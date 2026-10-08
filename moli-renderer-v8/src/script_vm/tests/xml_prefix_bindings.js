(async () => {
  const checks = [];
  const xmlns = 'http://www.w3.org/2000/xmlns/';
  const xml = 'http://www.w3.org/XML/1998/namespace';
  const xlink = 'http://www.w3.org/1999/xlink';
  const escape = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll('\t', '&#9;')
    .replaceAll('\n', '&#10;').replaceAll('\r', '&#13;');
  const record = (name, action, expected, callee) => {
    let actual;
    try { actual = action(); }
    catch (error) {
      actual = {name: error.name, code: error.code, realm: error instanceof callee.DOMException};
    }
    checks.push({name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected)});
  };
  const signature = node => ({
    namespace: node.namespaceURI,
    localName: node.localName,
    attributes: Array.from(node.attributes).filter(attr => attr.namespaceURI !== xmlns)
      .map(attr => [attr.namespaceURI, attr.localName, attr.value])
      .sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right))),
    children: Array.from(node.children).map(signature),
  });
  const cases = [
    ['explicit', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS('urn:a', 'p:v', v);
      return [root, '<root xmlns:p="urn:a" p:v="' + e + '"/>'];
    }],
    ['attribute-conflict', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS('urn:a', 'p:a', v);
      root.setAttributeNS('urn:b', 'p:b', v);
      return [root, '<root xmlns:p="urn:a" p:a="' + e + '" xmlns:ns1="urn:b" ns1:b="' + e + '"/>'];
    }],
    ['element-conflict', (doc, v, e) => {
      const root = doc.createElementNS('urn:element', 'p:root');
      root.setAttributeNS('urn:a', 'p:v', v);
      return [root, '<p:root xmlns:p="urn:element" xmlns:ns1="urn:a" ns1:v="' + e + '"/>'];
    }],
    ['local-conflict', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:declared');
      root.setAttributeNS('urn:a', 'p:v', v);
      return [root, '<root xmlns:p="urn:declared" xmlns:ns1="urn:a" ns1:v="' + e + '"/>'];
    }],
    ['ancestor-alternative', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:a');
      root.setAttributeNS(xmlns, 'xmlns:q', 'urn:a');
      const child = doc.createElementNS(null, 'child');
      child.setAttributeNS(xmlns, 'xmlns:q', 'urn:b');
      child.setAttributeNS('urn:a', 'v', v);
      root.appendChild(child);
      return [root, '<root xmlns:p="urn:a" xmlns:q="urn:a"><child xmlns:q="urn:b" p:v="' + e + '"/></root>'];
    }],
    ['ancestor-all-shadowed', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:a');
      const child = doc.createElementNS(null, 'child');
      child.setAttributeNS(xmlns, 'xmlns:p', 'urn:b');
      child.setAttributeNS('urn:a', 'v', v);
      root.appendChild(child);
      return [root, '<root xmlns:p="urn:a"><child xmlns:p="urn:b" xmlns:ns1="urn:a" ns1:v="' + e + '"/></root>'];
    }],
    ['ancestor-original-occupied', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:a');
      const child = doc.createElementNS(null, 'child');
      child.setAttributeNS('urn:b', 'p:v', v);
      root.appendChild(child);
      return [root, '<root xmlns:p="urn:a"><child xmlns:ns1="urn:b" ns1:v="' + e + '"/></root>'];
    }],
    ['generated-reserved', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:ns1', 'urn:reserved1');
      root.setAttributeNS(xmlns, 'xmlns:ns2', 'urn:reserved2');
      root.setAttributeNS('urn:a', 'v', v);
      root.setAttributeNS('urn:b', 'w', v);
      return [root, '<root xmlns:ns1="urn:reserved1" xmlns:ns2="urn:reserved2" xmlns:ns3="urn:a" ns3:v="' + e + '" xmlns:ns4="urn:b" ns4:w="' + e + '"/>'];
    }],
    ['generated-element-reserved', (doc, v, e) => {
      const root = doc.createElementNS('urn:element', 'p:root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:declared');
      root.setAttributeNS(xmlns, 'xmlns:ns1', 'urn:reserved');
      root.setAttributeNS('urn:a', 'p:v', v);
      return [root, '<ns2:root xmlns:ns2="urn:element" xmlns:p="urn:declared" xmlns:ns1="urn:reserved" xmlns:ns3="urn:a" ns3:v="' + e + '"/>'];
    }],
    ['sibling-scope', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:a');
      const child = doc.createElementNS(null, 'child');
      child.setAttributeNS(xmlns, 'xmlns:p', 'urn:b');
      child.setAttributeNS('urn:b', 'v', v);
      const sibling = doc.createElementNS(null, 'sibling');
      sibling.setAttributeNS('urn:a', 'v', v);
      root.append(child, sibling);
      return [root, '<root xmlns:p="urn:a"><child xmlns:p="urn:b" p:v="' + e + '"/><sibling p:v="' + e + '"/></root>'];
    }],
    ['element-rebinding', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:a');
      const child = doc.createElementNS('urn:b', 'p:child');
      child.setAttributeNS('urn:a', 'v', v);
      root.appendChild(child);
      return [root, '<root xmlns:p="urn:a"><p:child xmlns:p="urn:b" xmlns:ns1="urn:a" ns1:v="' + e + '"/></root>'];
    }],
    ['reserved-xml', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xml, 'p:lang', v);
      return [root, '<root xml:lang="' + e + '"/>'];
    }],
    ['xlink-element-conflict', (doc, v, e) => {
      const root = doc.createElementNS('urn:element', 'p:root');
      root.setAttributeNS(xlink, 'p:href', v);
      return [root, '<p:root xmlns:p="urn:element" xmlns:ns1="' + xlink + '" ns1:href="' + e + '"/>'];
    }],
    ['generated-attribute-conflict', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS('urn:a', 'v', v);
      root.setAttributeNS('urn:b', 'ns1:w', v);
      return [root, '<root xmlns:ns1="urn:a" ns1:v="' + e + '" xmlns:ns2="urn:b" ns2:w="' + e + '"/>'];
    }],
    ['attribute-value-rebinding', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:old');
      root.setAttributeNS(xmlns, 'xmlns:q', 'urn:old');
      const child = doc.createElementNS(null, 'child');
      child.setAttributeNS(xmlns, 'xmlns:q', 'urn:before');
      child.getAttributeNodeNS(xmlns, 'q').value = 'urn:new';
      child.setAttributeNS('urn:old', 'v', v);
      root.appendChild(child);
      return [root, '<root xmlns:p="urn:old" xmlns:q="urn:old"><child xmlns:q="urn:new" p:v="' + e + '"/></root>'];
    }],
    ['removed-declaration', (doc, v, e) => {
      const root = doc.createElementNS(null, 'root');
      root.setAttributeNS(xmlns, 'xmlns:p', 'urn:before');
      root.removeAttributeNS(xmlns, 'p');
      root.setAttributeNS('urn:a', 'p:v', v);
      return [root, '<root xmlns:p="urn:a" p:v="' + e + '"/>'];
    }],
  ];
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  let popup;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    if (globalThis.__xmlPrefixIncludePopup !== false) {
      popup = window.open('', '_blank');
      if (!popup) throw new Error('popup unavailable');
      await new Promise(resolve => setTimeout(resolve, 0));
      realms.push(['popup', popup]);
    }
    for (const [ownerName, owner] of realms) {
      const documents = [
        ['created-xml', owner.document.implementation.createDocument(null, '', null), true],
        ['parsed-xml', new owner.DOMParser().parseFromString('<document/>', 'application/xml'), true],
        ['created-html', owner.document.implementation.createHTMLDocument(''), false],
      ];
      for (const [docName, doc, isXml] of documents) {
        const copyDoc = owner.document.implementation.createDocument(null, '', null);
        for (const [caseName, build] of cases) {
          for (const [valueIndex, value] of ['text', '\t\n\r"&<>', '\uD83D\uDE00\uFFFD'].entries()) {
            const [source, expected] = build(doc, value, escape(value));
            const originalTree = signature(source);
            for (const [copyName, root] of [
              ['original', source], ['clone', source.cloneNode(true)], ['import', copyDoc.importNode(source, true)],
            ]) {
              for (const [calleeName, callee] of realms) {
                const name = [ownerName, docName, caseName, valueIndex, copyName, calleeName].join('/');
                record(name + '/serialize', () => new callee.XMLSerializer().serializeToString(root), expected, callee);
                record(name + '/round-trip', () => {
                  const output = new callee.XMLSerializer().serializeToString(root);
                  const parsed = new callee.DOMParser().parseFromString(output, 'application/xml');
                  const valid = parsed.getElementsByTagName('parsererror').length === 0;
                  return {valid, tree: signature(parsed.documentElement)};
                }, {valid: true, tree: originalTree}, callee);
                if (isXml || copyName === 'import') {
                  const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
                  record(name + '/checked-outer', () => outer.call(root), expected, callee);
                }
              }
              record([ownerName, docName, caseName, valueIndex, copyName, 'unmodified'].join('/'),
                () => signature(root), originalTree, owner);
            }
          }
        }
        for (const [valueIndex, value] of ['\uD800', '\uDC00'].entries()) {
          const root = doc.createElementNS(null, 'root');
          root.setAttributeNS('urn:a', 'p:v', value);
          for (const [calleeName, callee] of realms) {
            const name = [ownerName, docName, 'unpaired-value', valueIndex, calleeName].join('/');
            record(name + '/serialize', () => new callee.XMLSerializer().serializeToString(root),
              '<root xmlns:p="urn:a" p:v="' + value + '"/>', callee);
            if (isXml) {
              const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
              record(name + '/checked-outer', () => outer.call(root),
                {name: 'InvalidStateError', code: 11, realm: true}, callee);
            }
          }
        }
        const poisoned = doc.createElementNS(null, 'root');
        poisoned.setAttributeNS('urn:a', 'p:v', 'native');
        let reads = 0;
        for (const name of ['attributes', 'namespaceURI', 'localName', 'prefix', 'childNodes']) {
          Object.defineProperty(poisoned, name, {get() { reads++; throw new Error(name); }});
        }
        record(ownerName + '/' + docName + '/native-serialization',
          () => new owner.XMLSerializer().serializeToString(poisoned), '<root xmlns:p="urn:a" p:v="native"/>', owner);
        record(ownerName + '/' + docName + '/no-author-reads', () => reads, 0, owner);
      }
    }
    globalThis.__uiEventResults = {
      complete: true, total: checks.length, passed: checks.filter(row => row.passed).length,
      checks, includePopup: globalThis.__xmlPrefixIncludePopup !== false,
    };
    return globalThis.__uiEventResults.passed === checks.length;
  } finally {
    frame.remove();
    if (popup) popup.close();
  }
})()
