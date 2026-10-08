(async () => {
  const checks = [];
  const observe = (action, realm) => {
    try { return action(); }
    catch (error) { return { name: error.name, code: error.code, realm: error instanceof realm.DOMException }; }
  };
  const check = (name, action, expected, realm) => {
    const actual = observe(action, realm);
    checks.push({ name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected) });
  };
  const invalid = { name: 'InvalidStateError', code: 11, realm: true };
  const htmlNS = 'http://www.w3.org/1999/xhtml';
  const xmlnsNS = 'http://www.w3.org/2000/xmlns/';
  const xmlNS = 'http://www.w3.org/XML/1998/namespace';
  const escape = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
  const escapeAttr = value => escape(value).replaceAll('"', '&quot;').replaceAll('\t', '&#9;').replaceAll('\n', '&#10;').replaceAll('\r', '&#13;');
  const values = [
    ['', true], ['text &<>', true], ['\t\n\r', true], ['\uD7FF\uE000\uFFFD', true],
    ['\u{10000}\u{10FFFF}', true], ['\u007F\u0085\uFDD0\u{1FFFE}', true],
    ['\0', false], ['\f', false], ['\u001F', false], ['\uFFFE', false], ['\uFFFF', false],
    ['\uD800', false], ['\uDC00', false], ['\uD800\uD800', false], ['\uDC00\uD800', false],
    ['\uD83D\uDE00', true], ['a\uD800z', false],
  ];
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  let popup;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    if (globalThis.__xmlGetterIncludePopup !== false) {
      popup = window.open('', '_blank');
      if (!popup) throw new Error('popup unavailable');
      await new Promise(resolve => setTimeout(resolve, 0));
      realms.push(['popup', popup]);
    }
    for (const [ownerName, owner] of realms) {
      const docs = [
        ['xml', owner.document.implementation.createDocument(null, 'root')],
        ['parsed', new owner.DOMParser().parseFromString('<root/>', 'application/xml')],
        ['xhtml', new owner.DOMParser().parseFromString(`<html xmlns="${htmlNS}"/>`, 'application/xhtml+xml')],
      ];
      for (const [docName, doc] of docs) {
        const serializer = new owner.XMLSerializer();
        const root = () => doc.createElementNS(null, 'root');
        for (const [index, [value, valid]] of values.entries()) {
          const node = root();
          const text = doc.createTextNode(value);
          node.appendChild(text);
          const attribute = root();
          attribute.setAttribute('v', value);
          const comment = root();
          comment.appendChild(doc.createComment(value));
          const pi = root();
          const instruction = doc.createProcessingInstruction('target', '');
          instruction.data = value;
          pi.appendChild(instruction);
          const cdata = root();
          const section = doc.createCDATASection('');
          section.data = value;
          cdata.appendChild(section);
          const prefix = `${ownerName}/${docName}/char-${index}`;
          for (const [calleeName, callee] of realms) {
            const inner = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').get;
            const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
            check(`${prefix}/${calleeName}/text-inner`, () => inner.call(node), valid ? escape(value) : invalid, callee);
            check(`${prefix}/${calleeName}/text-outer`, () => outer.call(node), valid ? `<root>${escape(value)}</root>` : invalid, callee);
            check(`${prefix}/${calleeName}/attribute-outer`, () => outer.call(attribute), valid ? `<root v="${escapeAttr(value)}"/>` : invalid, callee);
            check(`${prefix}/${calleeName}/comment-inner`, () => inner.call(comment), valid ? `<!--${value}-->` : invalid, callee);
            check(`${prefix}/${calleeName}/pi-inner`, () => inner.call(pi), valid ? `<?target ${value}?>` : invalid, callee);
            // DOM Parsing's CDATASection algorithm does not inspect the flag.
            check(`${prefix}/${calleeName}/cdata-inner`, () => inner.call(cdata), `<![CDATA[${value}]]>`, callee);
          }
          check(`${prefix}/permissive-text`, () => serializer.serializeToString(node), `<root>${escape(value)}</root>`, owner);
          check(`${prefix}/permissive-attribute`, () => serializer.serializeToString(attribute), `<root v="${escapeAttr(value)}"/>`, owner);
          check(`${prefix}/preserved-data`, () => text.data, value, owner);
        }
        const cases = [
          ['local-colon', () => doc.createElement('a:b'), invalid, docName === 'xhtml' ? `<a:b xmlns="${htmlNS}"></a:b>` : '<a:b/>'],
          ['attribute-colon', () => { const node = root(); node.setAttribute('a:b', 'v'); return node; }, invalid, '<root a:b="v"/>'],
          ['namespace-character', () => doc.createElementNS('urn:\f', 'p:root'), invalid, '<p:root xmlns:p="urn:\f"/>'],
          ['prefixed-undeclaration', () => { const node = root(); node.setAttributeNS(xmlnsNS, 'xmlns:p', ''); return node; }, invalid, '<root xmlns:p=""/>'],
          ['reserved-namespace-declaration', () => { const node = root(); node.setAttributeNS(xmlnsNS, 'xmlns:p', xmlnsNS); return node; }, invalid, `<root xmlns:p="${xmlnsNS}"/>`],
          ['reserved-element-prefix', () => doc.createElementNS(xmlnsNS, 'xmlns:root'), invalid, `<xmlns:root xmlns:xmlns="${xmlnsNS}"/>`],
          ['namespace-declaration-character', () => { const node = root(); node.setAttributeNS(xmlnsNS, 'xmlns:p', 'urn:\uD800'); return node; }, invalid, '<root xmlns:p="urn:\uD800"/>'],
          ['skipped-default-declaration', () => { const node = root(); node.setAttributeNS(xmlnsNS, 'xmlns', 'urn:\uD800'); return node; }, '<root/>', '<root/>'],
          ['skipped-xml-declaration', () => { const node = root(); node.setAttributeNS(xmlnsNS, 'xmlns:p', xmlNS); return node; }, '<root/>', '<root/>'],
          ['default-undeclaration', () => { const node = doc.createElementNS('urn:parent', 'parent'); const child = root(); child.setAttributeNS(xmlnsNS, 'xmlns', ''); node.appendChild(child); return node; }, '<parent xmlns="urn:parent"><root xmlns=""/></parent>', '<parent xmlns="urn:parent"><root xmlns=""/></parent>'],
          ['split-surrogate-nodes', () => { const node = root(); node.append(doc.createTextNode('\uD83D'), doc.createTextNode('\uDE00')); return node; }, invalid, '<root>\uD83D\uDE00</root>'],
          ['comment-double-hyphen', () => { const node = root(); node.appendChild(doc.createComment('a--b')); return node; }, invalid, '<root><!--a--b--></root>'],
          ['comment-trailing-hyphen', () => { const node = root(); node.appendChild(doc.createComment('a-')); return node; }, invalid, '<root><!--a---></root>'],
          ['pi-xml', () => { const node = root(); node.appendChild(doc.createProcessingInstruction('XmL', '')); return node; }, invalid, '<root><?XmL ?></root>'],
          ['pi-colon', () => { const node = root(); node.appendChild(doc.createProcessingInstruction('a:b', '')); return node; }, invalid, '<root><?a:b ?></root>'],
          ['pi-close', () => { const node = root(); const pi = doc.createProcessingInstruction('target', ''); pi.data = 'a?>b'; node.appendChild(pi); return node; }, invalid, '<root><?target a?>b?></root>'],
          ['cdata-close', () => { const node = root(); const cdata = doc.createCDATASection(''); cdata.data = 'a]]>b'; node.appendChild(cdata); return node; }, '<root><![CDATA[a]]>b]]></root>', '<root><![CDATA[a]]>b]]></root>'],
        ];
        for (const [name, construct, expected, permissive] of cases) {
          const node = construct();
          for (const [calleeName, callee] of realms) {
            const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').get;
            check(`${ownerName}/${docName}/${name}/${calleeName}`, () => outer.call(node), expected, callee);
          }
          check(`${ownerName}/${docName}/${name}/permissive`, () => serializer.serializeToString(node), permissive, owner);
        }
        const poisoned = root();
        poisoned.appendChild(doc.createTextNode('\f'));
        let reads = 0;
        for (const name of ['nodeType', 'localName', 'namespaceURI', 'childNodes', 'textContent']) {
          Object.defineProperty(poisoned, name, { get() { reads++; throw new Error(name); } });
        }
        check(`${ownerName}/${docName}/native-validation`, () => poisoned.outerHTML, invalid, owner);
        check(`${ownerName}/${docName}/no-author-reads`, () => reads, 0, owner);
        poisoned.firstChild.data = 'fixed';
        check(`${ownerName}/${docName}/mutation-recovers`, () => poisoned.outerHTML, '<root>fixed</root>', owner);
        const host = doc.createElementNS(htmlNS, 'div');
        const shadow = host.attachShadow({ mode: 'open' });
        shadow.appendChild(doc.createTextNode('\f'));
        for (const [calleeName, callee] of realms) {
          const getter = Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, 'innerHTML').get;
          check(`${ownerName}/${docName}/shadow/${calleeName}`, () => getter.call(shadow), invalid, callee);
        }
      }
      const html = owner.document.implementation.createHTMLDocument('');
      const element = html.createElement('div');
      element.setAttribute('v', '\0\uD800');
      element.appendChild(html.createTextNode('\f\uDC00'));
      check(`${ownerName}/html-remains-permissive`, () => element.outerHTML, '<div v="\0\uD800">\f\uDC00</div>', owner);
    }
    globalThis.__uiEventResults = { complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks, includePopup: globalThis.__xmlGetterIncludePopup !== false };
    return globalThis.__uiEventResults.passed === checks.length;
  } finally {
    frame.remove();
    if (popup) popup.close();
  }
})()
