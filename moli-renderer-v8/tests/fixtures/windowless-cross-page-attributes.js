(() => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const popup = open();
  try {
    const factories = {
      html: realm => realm.document.implementation.createHTMLDocument(''),
      xml: realm => realm.document.implementation.createDocument(null, 'root'),
      parsed: realm => new realm.DOMParser().parseFromString('<body>', 'text/html'),
      template: realm => realm.document.createElement('template').content.ownerDocument,
    };
    const ns = 'urn:windowless-attributes';
    for (const [direction, source, target] of [
      ['to-popup', window, popup], ['from-popup', popup, window],
    ]) {
      for (const [kind, create] of Object.entries(factories)) {
        const prefix = `${direction}:${kind}:`;
        const element = create(source).createElementNS('http://www.w3.org/1999/xhtml', 'div');
        element.setAttribute('data-value', 'before\uD800');
        element.setAttributeNS(ns, 'p:value', 'before\uDC00');
        target.document.adoptNode(element);
        check(prefix + 'get', element.getAttribute('data-value') === 'before\uD800');
        check(prefix + 'get-ns', element.getAttributeNS(ns, 'value') === 'before\uDC00');
        check(prefix + 'names', element.getAttributeNames().join(',') === 'data-value,p:value');
        check(prefix + 'has', element.hasAttribute('data-value'));
        check(prefix + 'has-ns', element.hasAttributeNS(ns, 'value'));
        element.setAttribute('data-value', 'after\uD800');
        element.setAttributeNS(ns, 'p:value', 'after\uDC00');
        check(prefix + 'set', target.Element.prototype.getAttribute.call(element, 'data-value') === 'after\uD800');
        check(prefix + 'set-ns', target.Element.prototype.getAttributeNS.call(element, ns, 'value') === 'after\uDC00');
        element.removeAttribute('data-value');
        element.removeAttributeNS(ns, 'value');
        check(prefix + 'remove', !element.hasAttribute('data-value') && element.getAttribute('data-value') === null);
        check(prefix + 'remove-ns', !element.hasAttributeNS(ns, 'value') && element.getAttributeNS(ns, 'value') === null);
        element.setAttribute('data-return', 'retained\uD800');
        check(prefix + 'return', source.document.adoptNode(element) === element && element.getAttribute('data-return') === 'retained\uD800');
      }
    }
  } catch (error) {
    checks.push({name:'fixture-completed-without-exception',passed:false,details:String(error.stack || error)});
  } finally {
    popup?.close();
  }
  globalThis.__uiEventResults = {complete:true,checks,total:checks.length,passed:checks.filter(row => row.passed).length};
  return checks.every(row => row.passed);
})()
