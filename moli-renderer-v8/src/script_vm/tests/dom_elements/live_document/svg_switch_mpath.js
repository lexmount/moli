(() => {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const throws = (run, Constructor = TypeError) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    check(error instanceof Constructor, 'expected ' + Constructor.name + ', got ' + error);
  };
  const ns = 'http://www.w3.org/2000/svg';
  const xlink = 'http://www.w3.org/1999/xlink';
  const cases = [
    ['switch', SVGSwitchElement, SVGGraphicsElement],
    ['mpath', SVGMPathElement, SVGElement],
  ];
  const documents = [document, document.implementation.createHTMLDocument(),
    document.implementation.createDocument(ns, 'svg'),
    new DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
  for (const [tag, Constructor, Parent] of cases) {
    check(Constructor.length === 0 && Constructor.name === 'SVG' + (tag === 'switch' ? 'Switch' : 'MPath') + 'Element', tag + ' constructor metadata');
    check(Object.getPrototypeOf(Constructor) === Parent, tag + ' constructor inheritance');
    check(Object.getPrototypeOf(Constructor.prototype) === Parent.prototype, tag + ' prototype inheritance');
    throws(() => Constructor());
    throws(() => new Constructor());
    for (const doc of documents) {
      const element = doc.createElementNS(ns, 's:' + tag);
      check(Object.getPrototypeOf(element) === Constructor.prototype, tag + ' factory prototype');
      check(element instanceof Parent && element instanceof SVGElement, tag + ' native inheritance');
      check((element instanceof SVGGraphicsElement) === (tag === 'switch'), tag + ' graphics identity');
      check(Object.prototype.toString.call(element) === '[object ' + Constructor.name + ']', tag + ' native tag');
      check(element.prefix === 's' && element.localName === tag, tag + ' qualified name');
      check(Object.getPrototypeOf(element.cloneNode(true)) === Constructor.prototype, tag + ' clone');
      check(Object.getPrototypeOf(document.importNode(element, true)) === Constructor.prototype, tag + ' import');
      check(!(doc.createElementNS(ns, tag.toUpperCase()) instanceof Constructor), tag + ' case-sensitive factory');
      check(!(doc.createElementNS('urn:other', tag) instanceof Constructor), tag + ' namespace-sensitive factory');
      check(!(doc.createElement(tag) instanceof Constructor), tag + ' HTML factory');
      let events = 0;
      element.addEventListener('check', () => events++);
      element.dispatchEvent(new Event('check'));
      check(events === 1, tag + ' inherited EventTarget brand');
      if (tag === 'switch') {
        element.setAttribute('transform', 'translate(4 7)');
        check(element.transform.baseVal.getItem(0).matrix.e === 4, 'inherited live transform');
        element.systemLanguage.appendItem('en');
        check(element.getAttribute('systemLanguage') === 'en', 'inherited SVGTests list');
        check(!('href' in element), 'switch has no URI-reference mixin');
        continue;
      }
      const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, 'href');
      check(descriptor.enumerable && descriptor.configurable && !descriptor.set &&
        descriptor.get.name === 'get href' && descriptor.get.length === 0, 'href descriptor');
      const href = element.href;
      check(href instanceof SVGAnimatedString && href === element.href && !Object.hasOwn(element, 'href'), 'cached href object');
      check(href.baseVal === '' && href.animVal === '', 'missing href');
      const observer = new MutationObserver(() => {});
      observer.observe(element, {attributes: true, attributeOldValue: true});
      element.setAttributeNS(xlink, 'xlink:href', '#fallback');
      check(href.baseVal === '#fallback', 'xlink fallback');
      href.baseVal = '#changed';
      check(element.getAttributeNS(xlink, 'href') === '#changed' && !element.hasAttribute('href'), 'write existing xlink');
      element.setAttribute('href', '');
      check(href.animVal === '', 'empty href overrides xlink');
      let conversions = 0;
      href.baseVal = {toString() { conversions++; return '#preferred'; }};
      check(conversions === 1 && href.baseVal === '#preferred' && element.getAttribute('href') === '#preferred', 'convert and reflect href');
      check(element.getAttributeNS(xlink, 'href') === '#changed', 'preserve fallback attribute');
      const records = observer.takeRecords();
      check(records.length === 4 && records[1].attributeNamespace === xlink &&
        records[1].oldValue === '#fallback' && records[3].attributeNamespace === null &&
        records[3].oldValue === '', 'native mutation records');
      observer.disconnect();
      element.setAttribute('href', '#before-after');
      check(href.baseVal === '#before-after' && href.animVal === '#before-after', 'native attribute read');
      element.removeAttribute('href');
      check(href.baseVal === '#changed', 'removal reveals xlink');
      element.removeAttributeNS(xlink, 'href');
      check(href.baseVal === '' && href.animVal === '', 'remove both');
      check(!Reflect.set(href, 'animVal', '#ignored') && href.animVal === '', 'readonly animated value');
      const sentinel = {};
      try { href.baseVal = {toString() {throw sentinel;}}; throw Error('missing conversion exception'); }
      catch (error) {check(error === sentinel && !element.hasAttribute('href'), 'conversion exception leaves native state unchanged');}
      let traps = 0;
      const revoked = Proxy.revocable(element, {}); revoked.revoke();
      for (const receiver of [{}, Constructor.prototype, Object.create(element),
        new Proxy(element, {get() {traps++; throw Error('author trap');}}), revoked.proxy,
        doc.createElementNS(ns, 'use'), doc.createElementNS(ns, 'switch')]) {
        throws(() => descriptor.get.call(receiver));
      }
      check(traps === 0, 'brand check does not run author Proxy traps');
      Object.setPrototypeOf(element, null);
      check(descriptor.get.call(element) === href, 'native identity survives prototype mutation');
      href.baseVal = '#native';
      check(Element.prototype.getAttribute.call(element, 'href') === '#native', 'native mutation survives prototype mutation');
    }
  }
  const markup = '<svg xmlns="' + ns + '"><switch/><mpath href="#path"/></svg>';
  const xml = new DOMParser().parseFromString(markup, 'image/svg+xml');
  const html = document.createElement('div'); html.innerHTML = markup;
  for (const parent of [xml.documentElement, html.firstChild]) {
    check(parent.children[0] instanceof SVGSwitchElement && parent.children[1] instanceof SVGMPathElement, 'parser interfaces');
    check(parent.children[1].href.baseVal === '#path', 'parser href');
  }
  const frame = document.createElement('iframe'); document.body.append(frame);
  const other = frame.contentWindow;
  const foreign = other.document.createElementNS(ns, 'mpath');
  const getHref = Object.getOwnPropertyDescriptor(SVGMPathElement.prototype, 'href').get;
  const otherGetHref = Object.getOwnPropertyDescriptor(other.SVGMPathElement.prototype, 'href').get;
  const foreignHref = getHref.call(foreign);
  check(foreignHref instanceof other.SVGAnimatedString && !(foreignHref instanceof SVGAnimatedString), 'cached value belongs to element realm');
  check(foreign.href === foreignHref, 'cross-realm SameObject');
  throws(() => otherGetHref.call({}), other.TypeError);
  throws(() => otherGetHref.call(document.createElementNS(ns, 'use')), other.TypeError);
  const local = document.createElementNS(ns, 'mpath');
  check(otherGetHref.call(local) instanceof SVGAnimatedString, 'foreign accessor with local element');
  const imported = document.importNode(foreign, true);
  check(imported instanceof SVGMPathElement && !(imported instanceof other.SVGMPathElement), 'import selects destination realm');
  frame.remove();
  foreignHref.baseVal = '#retained';
  check(foreign.getAttribute('href') === '#retained', 'retained value after iframe removal');
  return true;
})()
