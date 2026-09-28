(() => {
  const check = (value, message) => {if (!value) throw new Error(message);};
  const svg = 'http://www.w3.org/2000/svg', xlink = 'http://www.w3.org/1999/xlink';
  const frame = document.createElement('iframe'); document.body.append(frame);
  const documents = [document, document.implementation.createHTMLDocument(),
    document.implementation.createDocument('urn:xml', 'root'), frame.contentDocument];
  const values = ['', 'plain', '\uD800', '\uDC00', '\uFFFD', 'x\uD800\uDC00\uDC00y'];
  for (const doc of documents) {
    for (const ns of [svg, 'http://www.w3.org/1999/xhtml', 'urn:xml']) {
      const element = doc.createElementNS(ns, 'item');
      for (const value of values) {
        let conversions = 0;
        element.setAttributeNS('urn:value', 'p:value', {toString() {conversions++; return value;}});
        check(conversions === 1, 'convert namespaced value once');
        check(element.getAttributeNS('urn:value', 'value') === value, 'namespace lookup preserves UTF-16');
        check(element.getAttribute('p:value') === value, 'qualified lookup preserves UTF-16');
        element.setAttributeNS(null, 'data-value', value);
        check(element.getAttribute('data-value') === value, 'null namespace write preserves UTF-16');
        element.setAttributeNS('', 'data-empty', value);
        check(element.getAttributeNS(null, 'data-empty') === value, 'empty namespace normalizes to null');
      }
      element.setAttributeNS('urn:first', 'p:shared', '\uD800');
      element.setAttributeNS('urn:second', 'p:shared', '\uDC00');
      check(element.getAttributeNS('urn:first', 'shared') === '\uD800' &&
        element.getAttributeNS('urn:second', 'shared') === '\uDC00', 'same qualified name has independent namespace values');
      element.setAttribute('p:shared', 'first\uD800');
      check(element.getAttributeNS('urn:first', 'shared') === 'first\uD800' &&
        element.getAttributeNS('urn:second', 'shared') === '\uDC00', 'qualified write changes only first matching attribute');
      for (const [operation, copy] of [['cloneNode', element.cloneNode()], ['importNode', document.importNode(element)]]) {
        check(copy.getAttributeNS('urn:first', 'shared') === 'first\uD800' &&
          copy.getAttributeNS('urn:second', 'shared') === '\uDC00', operation + ' preserves independent code units in document ' + documents.indexOf(doc) + ' / ' + ns);
      }
      element.setAttributeNS('urn:second', 'q:shared', '\uFFFD');
      check(element.getAttributeNS('urn:first', 'shared') === 'first\uD800', 'UTF-8 replacement clears only matching namespace units');
      element.removeAttributeNS('urn:second', 'shared');
      check(element.getAttributeNS('urn:first', 'shared') === 'first\uD800', 'namespace removal preserves other code units');
      element.setAttributeNS('urn:second', 'p:shared', '\uDC00');
      element.removeAttribute('p:shared');
      check(element.getAttributeNS('urn:first', 'shared') === null && element.getAttribute('p:shared') === '\uDC00', 'qualified removal reveals second namespace value');
      const sentinel = {};
      try {element.setAttributeNS('urn:second', 'p:shared', {toString() {throw sentinel;}}); throw Error('missing exception');}
      catch (error) {check(error === sentinel && element.getAttributeNS('urn:second', 'shared') === '\uDC00', 'conversion failure is atomic');}
      element.setAttributeNS('urn:second', 'p:shared', 'ordinary');
      check(element.getAttributeNS('urn:second', 'shared') === 'ordinary', 'ordinary write clears stale units');
    }
    for (const tag of ['use', 'feImage', 'mpath']) {
      const element = doc.createElementNS(svg, tag), href = element.href;
      for (const value of values) {
        href.baseVal = value;
        check(href.baseVal === value && href.animVal === value && element.getAttribute('href') === value, tag + ' href setter preserves UTF-16');
      }
      element.removeAttribute('href');
      element.setAttributeNS(xlink, 'ref:href', '#initial');
      href.baseVal = '#\uD800';
      check(!element.hasAttribute('href') && element.getAttributeNS(xlink, 'href') === '#\uD800', tag + ' update existing xlink attribute');
      element.setAttribute('href', '');
      href.baseVal = '#\uDC00';
      check(element.getAttribute('href') === '#\uDC00' && element.getAttributeNS(xlink, 'href') === '#\uD800', tag + ' independent href and xlink');
      element.removeAttribute('href');
      check(href.animVal === '#\uD800', tag + ' fallback preserves code units');
      href.baseVal = {toString() {element.setAttribute('href', '#created-during-conversion'); return '#\uDC00';}};
      check(element.getAttribute('href') === '#\uDC00' && element.getAttributeNS(xlink, 'href') === '#\uD800', tag + ' reevaluate namespace after conversion');
    }
  }
  const element = document.createElementNS(svg, 'g'); document.body.append(element);
  element.setAttributeNS('urn:adopt', 'p:adopt', {toString() {frame.contentDocument.adoptNode(element); return '\uD800';}});
  check(element.ownerDocument === frame.contentDocument && element.getAttributeNS('urn:adopt', 'adopt') === '\uD800', 'value conversion may adopt receiver');
  element.setAttributeNS({toString() {document.adoptNode(element); return 'urn:adopt';}}, 'p:adopt', '\uDC00');
  check(element.ownerDocument === document && element.getAttributeNS('urn:adopt', 'adopt') === '\uDC00', 'namespace conversion may adopt receiver');
  const setter = Element.prototype.setAttributeNS;
  let conversions = 0;
  try {setter.call(new Proxy(element, {}), null, 'data-value', {toString() {conversions++; return 'bad';}}); throw Error('missing brand exception');}
  catch (error) {check(error instanceof TypeError && conversions === 0, 'receiver check precedes conversion');}
  frame.remove();
  return true;
})()
