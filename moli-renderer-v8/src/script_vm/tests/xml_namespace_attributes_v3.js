(async () => {
  const checks = [];
  const xmlnsNS = 'http://www.w3.org/2000/xmlns/';
  const htmlNS = 'http://www.w3.org/1999/xhtml';
  const xmlNS = 'http://www.w3.org/XML/1998/namespace';
  const invalid = {name: 'InvalidStateError', code: 11, realm: true};
  const escape = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;').replaceAll('\t', '&#9;').replaceAll('\n', '&#10;').replaceAll('\r', '&#13;');
  const record = (name, action, expected, callee) => {
    let actual;
    try { actual = action(); }
    catch (error) { actual = {name: error.name, code: error.code, realm: error instanceof callee.DOMException}; }
    checks.push({name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected)});
  };
  const setters = [
    ['attribute', (doc, node, value) => node.setAttribute('xmlns', value)],
    ['attribute-value', (doc, node, value) => { node.setAttribute('xmlns', 'before'); node.getAttributeNodeNS(null, 'xmlns').value = value; }],
    ['attribute-node', (doc, node, value) => {
      const attr = doc.createAttribute('xmlns'); attr.value = value; node.setAttributeNode(attr);
    }],
    ['attribute-node-ns', (doc, node, value) => {
      const attr = doc.createAttribute('xmlns'); attr.value = value; node.setAttributeNodeNS(attr);
    }],
  ];
  const values = ['', 'urn:ordinary', 'urn:element', '\t\n\r"&<>', 'urn:\uD800', 'urn:\uDC00', 'urn:\uD83D\uDE00', 'urn:\f', xmlNS, xmlnsNS];
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  let popup;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    if (globalThis.__xmlNamespaceIncludePopup !== false) {
      popup = window.open('', '_blank');
      if (!popup) throw new Error('popup unavailable');
      await new Promise(resolve => setTimeout(resolve, 0));
      realms.push(['popup', popup]);
    }
    for (const [ownerName, owner] of realms) {
      const docs = [
        ['live', owner.document, false],
        ['html', owner.document.implementation.createHTMLDocument(''), false],
        ['xml', owner.document.implementation.createDocument(null, 'root'), true],
        ['parsed', new owner.DOMParser().parseFromString('<root/>', 'application/xml'), true],
        ['xhtml', new owner.DOMParser().parseFromString(`<html xmlns="${htmlNS}"/>`, 'application/xhtml+xml'), true],
      ];
      for (const [docName, doc, isXml] of docs) {
        if (globalThis.__xmlNamespaceScenario !== undefined &&
            globalThis.__xmlNamespaceScenario !== ownerName + '/' + docName) continue;
        for (const [index, value] of values.entries()) {
          const escaped = escape(value);
          const htmlEscaped = value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
          for (const [setterName, set] of setters) {
            const node = doc.createElementNS(null, 'root');
            set(doc, node, value);
            const attr = node.getAttributeNodeNS(null, 'xmlns');
            const parent = doc.createElementNS(null, 'parent');
            parent.appendChild(node);
            const name = `${ownerName}/${docName}/${index}/${setterName}`;
            for (const [calleeName, callee] of realms) {
              record(`${name}/${calleeName}/serialize`, () => new callee.XMLSerializer().serializeToString(node), `<root xmlns="${escaped}"/>`, callee);
              const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
              const inner = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').get;
              record(`${name}/${calleeName}/outer`, () => outer.call(node), isXml ? invalid : `<root xmlns="${htmlEscaped}"></root>`, callee);
              record(`${name}/${calleeName}/inner`, () => inner.call(parent), isXml ? invalid : `<root xmlns="${htmlEscaped}"></root>`, callee);
            }
            record(`${name}/namespace`, () => attr.namespaceURI, null, owner);
            record(`${name}/value`, () => attr.value, value, owner);
            record(`${name}/identity`, () => node.getAttributeNodeNS(null, 'xmlns') === attr && attr.ownerElement === node, true, owner);
            const clone = node.cloneNode(true);
            record(`${name}/clone-serialize`, () => new owner.XMLSerializer().serializeToString(clone), `<root xmlns="${escaped}"/>`, owner);
            record(`${name}/clone-value`, () => clone.getAttributeNodeNS(null, 'xmlns').value, value, owner);
            const imported = docs[2][1].importNode(node, true);
            record(`${name}/import-serialize`, () => new owner.XMLSerializer().serializeToString(imported), `<root xmlns="${escaped}"/>`, owner);
            record(`${name}/import-value`, () => imported.getAttributeNodeNS(null, 'xmlns').value, value, owner);
            attr.value = 'updated';
            record(`${name}/mutation`, () => new owner.XMLSerializer().serializeToString(node), '<root xmlns="updated"/>', owner);
            record(`${name}/clone-independent`, () => clone.getAttributeNodeNS(null, 'xmlns').value, value, owner);
          }
          const scenarios = [
            ['namespaced', 'urn:element', 'root', (node) => {}, `<root xmlns="urn:element" xmlns="${escaped}"/>`],
            ['prefixed', 'urn:element', 'p:root', (node) => {}, `<p:root xmlns:p="urn:element" xmlns="${escaped}"/>`],
            ['nested-reset', null, 'root', (node) => {
              const parent = doc.createElementNS('urn:element', 'parent'); parent.appendChild(node); return parent;
            }, `<parent xmlns="urn:element"><root xmlns="" xmlns="${escaped}"/></parent>`],
            ['nested-same', 'urn:element', 'root', (node) => {
              const parent = doc.createElementNS('urn:element', 'parent'); parent.appendChild(node); return parent;
            }, `<parent xmlns="urn:element"><root xmlns="${escaped}"/></parent>`],
            ['prefixed-parent', 'urn:element', 'p:root', (node) => {
              node.appendChild(doc.createElementNS(null, 'child'));
            }, `<p:root xmlns:p="urn:element" xmlns="${escaped}"><child/></p:root>`],
            ['real-default', null, 'root', (node) => {
              node.setAttributeNS(xmlnsNS, 'xmlns', 'urn:declared');
            }, `<root xmlns="${escaped}"/>`],
            ['real-matching-default', 'urn:element', 'root', (node) => {
              node.setAttributeNS(xmlnsNS, 'xmlns', 'urn:element');
            }, `<root xmlns="${escaped}" xmlns="urn:element"/>`],
          ];
          for (const [scenario, ns, qualifiedName, setup, expected] of scenarios) {
            const node = doc.createElementNS(ns, qualifiedName);
            node.setAttribute('xmlns', value);
            const root = setup(node) || node;
            const name = `${ownerName}/${docName}/${index}/${scenario}`;
            for (const [calleeName, callee] of realms) {
              record(`${name}/${calleeName}/serialize`, () => new callee.XMLSerializer().serializeToString(root), expected, callee);
              if (isXml) {
                const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
                record(`${name}/${calleeName}/outer`, () => outer.call(root), invalid, callee);
              }
            }
            record(`${name}/ordinary-value`, () => node.getAttributeNodeNS(null, 'xmlns').value, value, owner);
            record(`${name}/ordinary-namespace`, () => node.getAttributeNodeNS(null, 'xmlns').namespaceURI, null, owner);
          }
          const ordinaryPrefix = doc.createElementNS(null, 'root');
          ordinaryPrefix.setAttribute('xmlns:p', value);
          ordinaryPrefix.setAttributeNS('urn:attribute', 'v', 'text');
          for (const [calleeName, callee] of realms) {
            record(`${ownerName}/${docName}/${index}/${calleeName}/ordinary-prefix`, () => new callee.XMLSerializer().serializeToString(ordinaryPrefix), `<root xmlns:p="${escaped}" xmlns:ns1="urn:attribute" ns1:v="text"/>`, callee);
          }
        }
        const rejected = {name: 'NamespaceError', code: 14, realm: true};
        record(`${ownerName}/${docName}/reject-null-namespace-set`, () => doc.createElementNS(null, 'root').setAttributeNS(null, 'xmlns', 'value'), rejected, owner);
        record(`${ownerName}/${docName}/reject-null-namespace-create`, () => doc.createAttributeNS(null, 'xmlns'), rejected, owner);
        const control = doc.createElementNS(null, 'root');
        control.setAttributeNS(xmlnsNS, 'xmlns', 'urn:discarded');
        for (const [calleeName, callee] of realms) {
          record(`${ownerName}/${docName}/${calleeName}/actual-declaration`, () => new callee.XMLSerializer().serializeToString(control), '<root/>', callee);
          if (isXml) {
            const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
            record(`${ownerName}/${docName}/${calleeName}/actual-declaration-outer`, () => outer.call(control), '<root/>', callee);
          }
        }
        const poisoned = doc.createElementNS(null, 'root');
        poisoned.setAttribute('xmlns', 'native');
        let reads = 0;
        for (const name of ['attributes', 'localName', 'namespaceURI', 'prefix', 'childNodes']) {
          Object.defineProperty(poisoned, name, {get() { reads++; throw new Error(name); }});
        }
        record(`${ownerName}/${docName}/native-serialization`, () => new owner.XMLSerializer().serializeToString(poisoned), '<root xmlns="native"/>', owner);
        record(`${ownerName}/${docName}/native-no-author-reads`, () => reads, 0, owner);
      }
    }
    globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks, includePopup: globalThis.__xmlNamespaceIncludePopup !== false};
    return globalThis.__uiEventResults.passed === checks.length;
  } finally {
    frame.remove();
    if (popup) popup.close();
  }
})()
