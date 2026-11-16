(() => {
  const other = document.getElementById('child').contentWindow;
  const documents = [
    ['main', document, Attr],
    ['iframe', other.document, other.Attr],
    ['windowless', document.implementation.createHTMLDocument(''), Attr],
    ['xml', document.implementation.createDocument(null, 'root'), Attr]
  ];
  const rows = [];
  function check(name, expected, run) {
    let actual;
    try { actual = run(); } catch (error) { actual = {error: error.name, message: error.message}; }
    rows.push({name, expected, actual, pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  const removals = [
    ['toggle-false', 'data-x', el => el.toggleAttribute('data-x', false)],
    ['toggle', 'data-x', el => el.toggleAttribute('data-x')],
    ['dataset', 'data-x', el => { delete el.dataset.x; }],
    ['remove', 'data-x', el => el.removeAttribute('data-x')],
    ['remove-ns', 'data-x', el => el.removeAttributeNS(null, 'data-x')],
    ['remove-node', 'data-x', (el, attr) => el.removeAttributeNode(attr)],
    ['named-map', 'data-x', el => el.attributes.removeNamedItem('data-x')],
    ['named-map-ns', 'data-x', el => el.attributes.removeNamedItemNS(null, 'data-x')],
    ['reflection', 'hidden', el => { el.hidden = false; }]
  ];
  for (const [context, doc, AttrConstructor] of documents) {
    for (const [operation, name, remove] of removals) {
      check(context + '/removal/' + operation,
        [null, true, 'latest', 'latest', 'latest', 'latest', true, 33, 33, true, true, 'latest', null, 'replacement', true, 'detached'],
        () => {
          const el = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
          el.setAttribute(name, 'initial');
          const attr = el.getAttributeNode(name);
          el.setAttribute(name, 'latest');
          remove(el, attr);
          const result = [attr.ownerElement, attr.ownerDocument === doc, attr.value, attr.nodeValue,
            attr.textContent, attr.cloneNode().value, attr.cloneNode() instanceof AttrConstructor,
            el.compareDocumentPosition(attr) & 33, attr.compareDocumentPosition(el) & 33];
          el.setAttribute(name, 'replacement');
          const replacement = el.getAttributeNode(name);
          result.push(replacement !== attr, replacement.ownerElement === el, attr.value, attr.ownerElement);
          attr.value = 'detached';
          result.push(el.getAttribute(name));
          el.removeAttribute(name);
          el.setAttributeNode(attr);
          result.push(el.getAttributeNodeNS(null, name) === attr, el.getAttribute(name));
          return result;
        });
    }
    for (const operation of ['removeAttribute', 'removeAttributeNS']) {
      check(context + '/namespaced-removal/' + operation, [null, 'latest', true, 33, 33], () => {
        const el = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        el.setAttributeNS('urn:attr', 'p:key', 'initial');
        const attr = el.getAttributeNodeNS('urn:attr', 'key');
        el.setAttributeNS('urn:attr', 'p:key', 'latest');
        if (operation === 'removeAttribute') el.removeAttribute('p:key');
        else el.removeAttributeNS('urn:attr', 'key');
        return [attr.ownerElement, attr.value, attr.ownerDocument === doc,
          el.compareDocumentPosition(attr) & 33, attr.compareDocumentPosition(el) & 33];
      });
    }
    for (const operation of ['setAttributeNode', 'setAttributeNodeNS']) {
      check(context + '/mixed-case/' + operation,
        ['DATA-X', 'DATA-X', 'DATA-X', true, 'new', 'new', 'new', 'new', 'lower', 0, 0, true, null, 'new', 'lower', 0],
        () => {
          const el = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
          const attr = doc.createAttributeNS(null, 'DATA-X');
          attr.value = 'old';
          el[operation](attr);
          const nativeName = el.getAttributeNames()[0];
          el.setAttributeNS(null, nativeName, 'new');
          el.setAttributeNS(null, 'data-x', 'lower');
          const lower = el.getAttributeNodeNS(null, 'data-x');
          let publicCalls = 0;
          for (const method of ['getAttribute', 'getAttributeNS']) {
            Object.defineProperty(el, method, {value() { publicCalls++; throw Error('author method'); }});
          }
          const result = [attr.name, attr.localName, nativeName,
            el.getAttributeNodeNS(null, 'DATA-X') === attr,
            attr.value, attr.nodeValue, attr.textContent, attr.cloneNode().value, lower.value,
            attr.compareDocumentPosition(lower) & 1, lower.compareDocumentPosition(attr) & 1];
          result.push(el.removeAttributeNode(attr) === attr, attr.ownerElement, attr.value, lower.value, publicCalls);
          return result;
        });
    }
    for (const [namespace, name] of [[null, 'PLAIN'], ['urn:attr', 'p:owned']]) {
      check(context + '/adoption-clone/' + name, [true, true, true], () => {
        const el = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        const attr = doc.createAttributeNS(namespace, name);
        attr.value = 'value';
        el.setAttributeNode(attr);
        const nextDocument = context === 'iframe' ? document.implementation.createHTMLDocument('') :
          other.document.implementation.createHTMLDocument('');
        nextDocument.adoptNode(el);
        return [attr, Node.prototype, other.Node.prototype].map(api => {
          const clone = api.cloneNode.call(attr);
          return clone !== attr && clone.ownerDocument === nextDocument && clone.ownerElement === null &&
            clone instanceof AttrConstructor && Object.getPrototypeOf(clone) === AttrConstructor.prototype &&
            clone.name === name && clone.namespaceURI === namespace && clone.value === 'value';
        });
      });
    }
  }
  check('repeated-adoption-and-detachment', [true, true, true, true], () => {
    const el = other.document.createElement('div');
    const attr = other.document.createAttributeNS(null, 'DATA-X');
    attr.value = 'value';
    el.setAttributeNode(attr);
    const targets = [document.implementation.createHTMLDocument(''),
      other.document.implementation.createHTMLDocument(''),
      document.implementation.createDocument(null, 'root')];
    const result = targets.map((target, index) => {
      target.adoptNode(el);
      el.setAttributeNS(null, 'DATA-X', String(index));
      const clone = Node.prototype.cloneNode.call(attr);
      return attr.value === String(index) && clone.value === String(index) &&
        clone.ownerDocument === target && Object.getPrototypeOf(clone) === other.Attr.prototype;
    });
    el.removeAttributeNS(null, 'DATA-X');
    const clone = attr.cloneNode();
    result.push(attr.ownerElement === null && attr.ownerDocument === targets[2] && attr.value === '2' &&
      clone.ownerDocument === targets[2] && Object.getPrototypeOf(clone) === other.Attr.prototype);
    return result;
  });
  check('removal-detaches-before-custom-element-reaction', [true, 'latest', 33, true, 'replacement'], () => {
    let attr, during;
    customElements.define('attr-reference-removal', class extends HTMLElement {
      static get observedAttributes() { return ['data-x']; }
      attributeChangedCallback(name, oldValue, newValue) {
        if (newValue === null) {
          during = [attr.ownerElement === null, attr.value, this.compareDocumentPosition(attr) & 33];
          this.setAttribute(name, 'replacement');
        }
      }
    });
    const el = document.createElement('attr-reference-removal');
    el.setAttribute('data-x', 'latest');
    attr = el.getAttributeNode('data-x');
    el.toggleAttribute('data-x', false);
    return during.concat([el.getAttributeNode('data-x') !== attr, el.getAttribute('data-x')]);
  });
  globalThis.__attrReferenceRegressionFailures = rows.filter(row => !row.pass);
  globalThis.__attrReferenceRegressionResults = {total: rows.length, rows};
  return __attrReferenceRegressionFailures.length === 0;
})()
