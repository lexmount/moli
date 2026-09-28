(() => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const frame = document.createElement('iframe');document.body.append(frame);
  for (const doc of [document, document.implementation.createHTMLDocument(),
    document.implementation.createDocument('urn:xml', 'root'), frame.contentDocument]) {
    for (const ns of ['http://www.w3.org/1999/xhtml', 'http://www.w3.org/2000/svg', 'urn:xml']) {
      const element = doc.createElementNS(ns, 'item');
      if (doc.documentElement) doc.documentElement.appendChild(element);
      for (const value of ['', 'plain', '\uD800', '\uDC00', '\uFFFD', 'x\uD800\uDC00\uDC00y']) {
        let conversions = 0;
        element.setAttribute('data-value', {toString() {conversions++;return value;}});
        check(conversions === 1, 'convert attribute value once');
        check(element.getAttribute('data-value') === value, 'qualified-name lookup preserves code units');
        check(element.getAttributeNS(null, 'data-value') === value, 'namespace lookup preserves code units');
      }
      element.setAttributeNS('urn:attr', 'p:name', 'initial');
      element.setAttribute('p:name', '\uD800');
      check(element.getAttribute('p:name') === '\uD800', 'qualified write to namespaced attribute');
      check(element.getAttributeNS('urn:attr', 'name') === '\uD800', 'namespace lookup uses original units');
      element.setAttribute('data-value', 'before');
      const exception = {};
      let caught;
      try {element.setAttribute('data-value', {toString() {throw exception;}});} catch (e) {caught=e;}
      check(caught === exception && element.getAttribute('data-value') === 'before', 'conversion failure preserves attribute');
      element.removeAttribute('data-value');
      check(element.getAttribute('data-value') === null, 'missing differs from empty');
      element.remove();
      element.setAttribute('data-value', '\uDC00');
      check(element.getAttribute('data-value') === '\uDC00', 'removed native node keeps string units');
    }
  }
  const element = document.createElementNS('http://www.w3.org/2000/svg', 'g');
  document.body.appendChild(element);
  element.setAttribute('data-value', {toString() {frame.contentDocument.adoptNode(element);return '\uD800';}});
  check(element.ownerDocument === frame.contentDocument && element.getAttribute('data-value') === '\uD800', 'value conversion may adopt receiver');
  const value = element.getAttribute({toString() {document.adoptNode(element);return 'data-value';}});
  check(element.ownerDocument === document && value === '\uD800', 'name conversion may adopt receiver');
  frame.remove();
  return true;
})()
