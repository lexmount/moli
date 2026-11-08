(() => {
  const checks = [];
  const xmlnsNS = 'http://www.w3.org/2000/xmlns/';
  const record = (name, action, expected) => {
    let actual;
    try { actual = action(); } catch (error) { actual = error.name; }
    checks.push({name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected)});
  };
  const docs = [
    ['live', document],
    ['html', document.implementation.createHTMLDocument('')],
    ['xml', document.implementation.createDocument(null, 'root')],
  ];
  const setters = [
    ['attribute', (doc, node, value) => node.setAttribute('xmlns', value)],
    ['attribute-value', (doc, node, value) => {
      node.setAttribute('xmlns', 'before'); node.getAttributeNodeNS(null, 'xmlns').value = value;
    }],
    ['attribute-node', (doc, node, value) => {
      const attr = doc.createAttribute('xmlns'); attr.value = value; node.setAttributeNode(attr);
    }],
    ['attribute-node-ns', (doc, node, value) => {
      const attr = doc.createAttribute('xmlns'); attr.value = value; node.setAttributeNodeNS(attr);
    }],
  ];
  for (const [docName, doc] of docs) {
    for (const value of ['', 'urn:ordinary', 'urn:element']) {
      for (const [setterName, set] of setters) {
        const node = doc.createElementNS(null, 'root');
        set(doc, node, value);
        const attr = node.getAttributeNodeNS(null, 'xmlns');
        const name = `${docName}/${value}/${setterName}`;
        record(`${name}/serialize`, () => new XMLSerializer().serializeToString(node), `<root xmlns="${value}"/>`);
        record(`${name}/namespace`, () => attr.namespaceURI, null);
        record(`${name}/identity`, () => node.getAttributeNodeNS(null, 'xmlns') === attr && attr.ownerElement === node, true);
        const clone = node.cloneNode(true);
        record(`${name}/clone`, () => new XMLSerializer().serializeToString(clone), `<root xmlns="${value}"/>`);
        const imported = docs[2][1].importNode(node, true);
        record(`${name}/import`, () => new XMLSerializer().serializeToString(imported), `<root xmlns="${value}"/>`);
        attr.value = 'updated';
        record(`${name}/mutation`, () => new XMLSerializer().serializeToString(node), '<root xmlns="updated"/>');
        record(`${name}/clone-independent`, () => clone.getAttributeNodeNS(null, 'xmlns').value, value);
      }
      const scenarios = [
        ['namespaced', 'urn:element', 'root', () => {}, `<root xmlns="urn:element" xmlns="${value}"/>`],
        ['prefixed', 'urn:element', 'p:root', () => {}, `<p:root xmlns:p="urn:element" xmlns="${value}"/>`],
        ['nested-reset', null, 'root', node => {
          const parent = doc.createElementNS('urn:element', 'parent'); parent.appendChild(node); return parent;
        }, `<parent xmlns="urn:element"><root xmlns="" xmlns="${value}"/></parent>`],
        ['nested-same', 'urn:element', 'root', node => {
          const parent = doc.createElementNS('urn:element', 'parent'); parent.appendChild(node); return parent;
        }, `<parent xmlns="urn:element"><root xmlns="${value}"/></parent>`],
        ['prefixed-parent', 'urn:element', 'p:root', node => {
          node.appendChild(doc.createElementNS(null, 'child'));
        }, `<p:root xmlns:p="urn:element" xmlns="${value}"><child/></p:root>`],
        ['real-default', null, 'root', node => {
          node.setAttributeNS(xmlnsNS, 'xmlns', 'urn:declared');
        }, `<root xmlns="${value}"/>`],
        ['real-matching-default', 'urn:element', 'root', node => {
          node.setAttributeNS(xmlnsNS, 'xmlns', 'urn:element');
        }, `<root xmlns="${value}" xmlns="urn:element"/>`],
      ];
      for (const [name, ns, qualifiedName, setup, expected] of scenarios) {
        const node = doc.createElementNS(ns, qualifiedName);
        node.setAttribute('xmlns', value);
        const root = setup(node) || node;
        record(`${docName}/${value}/${name}`, () => new XMLSerializer().serializeToString(root), expected);
      }
    }
    const declared = doc.createElementNS('urn:element', 'p:root');
    declared.setAttributeNS(xmlnsNS, 'xmlns', 'urn:declared');
    declared.appendChild(doc.createElementNS(null, 'child'));
    record(`${docName}/real-default-resets-child`, () => new XMLSerializer().serializeToString(declared), '<p:root xmlns:p="urn:element" xmlns="urn:declared"><child xmlns=""/></p:root>');
    const poisoned = doc.createElementNS(null, 'root');
    poisoned.setAttribute('xmlns', 'native');
    let reads = 0;
    for (const name of ['attributes', 'localName', 'namespaceURI', 'prefix', 'childNodes']) {
      Object.defineProperty(poisoned, name, {get() { reads++; throw new Error(name); }});
    }
    record(`${docName}/native-serialization`, () => new XMLSerializer().serializeToString(poisoned), '<root xmlns="native"/>');
    record(`${docName}/native-no-author-reads`, () => reads, 0);
  }
  globalThis.__ordinaryXmlnsChecks = checks;
  return checks.every(row => row.passed);
})()
