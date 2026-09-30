(() => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const throws = (callback, name, Constructor = Error) => {
    let error;
    try { callback(); } catch (caught) { error = caught; }
    check(error && error.name === name && error instanceof Constructor, name + ' required');
  };
  const ns = 'http://www.w3.org/2000/svg';
  const xlink = 'http://www.w3.org/1999/xlink';
  const cases = [
    ['feComponentTransfer', SVGFEComponentTransferElement, ['in1']],
    ['feFlood', SVGFEFloodElement, []],
    ['feImage', SVGFEImageElement, ['href', 'preserveAspectRatio']],
    ['feMerge', SVGFEMergeElement, []],
    ['feMergeNode', SVGFEMergeNodeElement, ['in1']],
    ['feTile', SVGFETileElement, ['in1']],
  ];
  const documents = [document, document.implementation.createHTMLDocument(),
    document.implementation.createDocument(ns, 'svg'),
    new DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
  const standard = ['x', 'y', 'width', 'height', 'result'];
  for (const [tag, Constructor, extra] of cases) {
    check(Constructor.length === 0, tag + ' constructor length');
    check(Object.getPrototypeOf(Constructor) === SVGElement, tag + ' constructor inheritance');
    check(Object.getPrototypeOf(Constructor.prototype) === SVGElement.prototype, tag + ' prototype inheritance');
    throws(() => Constructor(), 'TypeError', TypeError);
    throws(() => new Constructor(), 'TypeError', TypeError);
    const attributes = [...(tag === 'feMergeNode' ? [] : standard), ...extra];
    for (const doc of documents) {
      const element = doc.createElementNS(ns, tag);
      check(Object.getPrototypeOf(element) === Constructor.prototype, tag + ' factory prototype');
      check(Object.prototype.toString.call(element) === '[object ' + Constructor.name + ']', tag + ' native tag');
      check(element instanceof SVGElement && !(element instanceof SVGGraphicsElement), tag + ' native inheritance');
      check(Object.getPrototypeOf(element.cloneNode()) === Constructor.prototype, tag + ' clone interface');
      check(Object.getPrototypeOf(document.importNode(element)) === Constructor.prototype, tag + ' import interface');
      check(!(doc.createElementNS(ns, tag.toUpperCase()) instanceof Constructor), tag + ' case-sensitive namespace factory');
      check(!(doc.createElementNS('urn:other', tag) instanceof Constructor), tag + ' namespace-sensitive factory');
      let traps = 0;
      const revoked = Proxy.revocable(element, {});revoked.revoke();
      for (const name of attributes) {
        const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, name);
        check(descriptor && descriptor.enumerable && descriptor.configurable && !descriptor.set, tag + '.' + name + ' readonly descriptor');
        check(descriptor.get.name === 'get ' + name && descriptor.get.length === 0, tag + '.' + name + ' getter metadata');
        const value = element[name];
        check(value === element[name] && !Object.hasOwn(element, name), tag + '.' + name + ' stable prototype accessor');
        for (const receiver of [{}, Constructor.prototype, Object.create(element),
          new Proxy(element, {get() {traps++;throw Error('author trap');}}), revoked.proxy,
          doc.createElementNS(ns, tag === 'feTile' ? 'feFlood' : 'feTile')]) {
          throws(() => descriptor.get.call(receiver), 'TypeError', TypeError);
        }
      }
      check(traps === 0, tag + ' brand check does not run Proxy traps');
      if (tag === 'feMergeNode') {
        for (const name of standard) check(!(name in element), 'merge node has no primitive region');
      } else {
        for (const [name, initial] of [['x','0%'],['y','0%'],['width','100%'],['height','100%']]) {
          const animated = element[name];
          check(animated instanceof SVGAnimatedLength, tag + '.' + name + ' animated length');
          check(animated.baseVal.valueAsString === initial && animated.animVal.valueAsString === initial, tag + '.' + name + ' initial');
          element.setAttribute(name, '12');
          check(animated.baseVal.value === 12 && animated.animVal.value === 12, tag + '.' + name + ' live attribute reflection');
          animated.baseVal.valueAsString = '25%';
          check(element.getAttribute(name) === '25%' && animated.animVal.valueAsString === '25%', tag + '.' + name + ' reflected setter');
          throws(() => {animated.animVal.value = 1;}, 'NoModificationAllowedError', DOMException);
          element.removeAttribute(name);
          check(animated.baseVal.valueAsString === initial, tag + '.' + name + ' removal restores default');
        }
      }
      for (const name of ['result', 'in1'].filter(name => attributes.includes(name))) {
        const attribute = name === 'in1' ? 'in' : name;
        const value = element[name];
        check(value instanceof SVGAnimatedString && value.baseVal === '' && value.animVal === '', tag + '.' + name + ' initial string');
        element.setAttribute(attribute, 'SourceGraphic');
        check(value.baseVal === 'SourceGraphic' && value.animVal === 'SourceGraphic', tag + '.' + name + ' cached reflection');
        let conversions = 0;
        value.baseVal = {toString() {conversions++;return 'SourceAlpha';}};
        check(conversions === 1 && element.getAttribute(attribute) === 'SourceAlpha' && value.animVal === 'SourceAlpha', tag + '.' + name + ' native mutation');
        element.removeAttribute(attribute);
        check(value.baseVal === '' && value.animVal === '', tag + '.' + name + ' string removal');
      }
      if (tag === 'feImage') {
        const href = element.href;
        element.setAttributeNS(xlink, 'xlink:href', '#fallback');
        check(href.baseVal === '#fallback', 'xlink href fallback');
        href.baseVal = '#changed';
        check(element.getAttributeNS(xlink, 'href') === '#changed' && !element.hasAttribute('href'), 'setter reflects existing xlink href');
        element.setAttribute('href', '#preferred');
        check(href.animVal === '#preferred', 'unprefixed href has priority');
        href.baseVal = '#updated';
        check(element.getAttribute('href') === '#updated' && element.getAttributeNS(xlink, 'href') === '#changed', 'setter follows href precedence');
        element.removeAttribute('href');
        check(href.baseVal === '#changed', 'removal restores xlink fallback');
        const ratio = element.preserveAspectRatio;
        check(ratio instanceof SVGAnimatedPreserveAspectRatio && ratio.baseVal.align === 6 && ratio.baseVal.meetOrSlice === 1, 'initial preserveAspectRatio');
        element.setAttribute('preserveAspectRatio', 'xMaxYMax slice');
        check(ratio.baseVal.align === 10 && ratio.animVal.meetOrSlice === 2, 'preserveAspectRatio live reflection');
        ratio.baseVal.align = 2;
        check(element.getAttribute('preserveAspectRatio') === 'xMinYMin slice', 'preserveAspectRatio reflected setter');
        throws(() => {ratio.animVal.align = 3;}, 'NoModificationAllowedError', DOMException);
      }
    }
  }
  const parsed = new DOMParser().parseFromString('<svg xmlns="' + ns + '">' + cases.map(([tag]) => '<' + tag + '/>').join('') + '</svg>', 'image/svg+xml');
  const html = document.createElement('div');
  html.innerHTML = '<svg>' + cases.map(([tag]) => '<' + tag.toLowerCase() + '/>').join('') + '</svg>';
  for (const [i, [, Constructor]] of cases.entries()) {
    check(parsed.documentElement.children[i] instanceof Constructor, 'XML parser interface ' + Constructor.name + ': ' + Object.prototype.toString.call(parsed.documentElement.children[i]) + ' ' + parsed.documentElement.innerHTML);
    check(html.firstChild.children[i] instanceof Constructor, 'HTML foreign-content case adjustment');
  }
  const frame = document.createElement('iframe');document.body.append(frame);
  const other = frame.contentWindow;
  for (const [tag, Constructor, extra] of cases) {
    const foreign = other.document.createElementNS(ns, tag);
    const attributes = [...(tag === 'feMergeNode' ? [] : standard), ...extra];
    for (const name of attributes) {
      const getter = Object.getOwnPropertyDescriptor(Constructor.prototype, name).get;
      const otherGetter = Object.getOwnPropertyDescriptor(other[Constructor.name].prototype, name).get;
      const value = getter.call(foreign);
      const ValueConstructor = ['x','y','width','height'].includes(name) ? 'SVGAnimatedLength' : name === 'preserveAspectRatio' ? 'SVGAnimatedPreserveAspectRatio' : 'SVGAnimatedString';
      check(value instanceof other[ValueConstructor] && !(value instanceof window[ValueConstructor]), tag + '.' + name + ' owner realm');
      throws(() => otherGetter.call({}), 'TypeError', other.TypeError);
    }
  }
  const kept = other.document.createElementNS(ns, 'feTile');
  const input = kept.in1;
  frame.remove();
  input.baseVal = 'after-removal';
  check(kept.getAttribute('in') === 'after-removal', 'retained filter value survives iframe removal');
  return true;
})()
