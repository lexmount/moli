(() => {
  const check = (ok, label) => { if (!ok) throw new Error(label); };
  const throws = (realm, run, label) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    check(error instanceof realm.TypeError, label);
  };
  const sources = [
    'linear-gradient(to right, red, blue)',
    'radial-gradient(circle at 25% 75%, red, blue)',
    'conic-gradient(from 45deg, red, blue)',
    'repeating-linear-gradient(red 0px, blue 10px)',
    'repeating-radial-gradient(red 0px, blue 10px)',
    'repeating-conic-gradient(red 0deg, blue 30deg)',
    'image-set(url("/one.png") 1x, linear-gradient(red, blue) 2x)',
    'image-set("data:image/svg+xml,<svg/>" 1x type("image/svg+xml"), url("/two.png") 2x)',
    String.raw`\6c inear-gradient(/**/red, blue)`,
    'linear-gradient(red)',
  ];
  const child = document.querySelector('iframe').contentWindow;
  for (const realm of [window, child]) {
    const element = realm.document.createElement('div');
    realm.document.body.append(element);
    const map = element.attributeStyleMap;
    const stringify = realm.CSSStyleValue.prototype.toString;
    const properties = ['background-image', 'mask-image', 'border-image-source', 'list-style-image'];
    for (const source of sources) {
      const value = realm.CSSStyleValue.parse('background-image', source);
      check(value instanceof realm.CSSImageValue, 'generated image type: ' + source);
      check(Object.getPrototypeOf(value) === realm.CSSImageValue.prototype, 'native image prototype');
      for (const property of properties) {
        element.style.setProperty(property, source);
        const text = element.style.getPropertyValue(property);
        check(text !== '', 'native parser accepts image');
        check(stringify.call(value) === text, 'native specified serialization');
        map.set(property, value);
        check(map.get(property) instanceof realm.CSSImageValue, 'cross-property image write');
        check(String(map.get(property)) === text, 'declared image serialization');
        const computed = element.computedStyleMap().get(property);
        check(computed instanceof realm.CSSImageValue, 'computed image type');
        check(String(computed) === realm.getComputedStyle(element).getPropertyValue(property), 'computed image serialization');
      }
    }

    const parts = [sources[0], 'none', sources[6], 'url("/last.png")', sources[1]];
    const list = realm.CSSStyleValue.parseAll('background-image', parts.join(', '));
    const verifyList = (items, label) => {
      check(items.length === parts.length, label + ': cardinality');
      items.forEach((value, i) => check(i === 1
        ? value instanceof realm.CSSKeywordValue && value.value === 'none'
        : value instanceof realm.CSSImageValue, label + ': iteration ' + i));
    };
    verifyList(list, 'parseAll');
    check(String(realm.CSSStyleValue.parse('background-image', parts.join(', '))) === String(list[0]), 'parse returns first iteration');
    for (const property of ['background-image', 'mask-image']) {
      map.set(property, ...list);
      verifyList(map.getAll(property), 'typed set');
      check(map.getAll(property).map(String).join(', ') === element.style.getPropertyValue(property), 'declared list order');
      const computed = element.computedStyleMap().getAll(property);
      verifyList(computed, 'computed list');
      check(computed.map(String).join(', ') === realm.getComputedStyle(element).getPropertyValue(property), 'computed list order');
      map.set(property, list[0]);
      map.append(property, ...list.slice(1));
      verifyList(map.getAll(property), 'append');
      const before = element.style.getPropertyValue(property);
      throws(realm, () => map.set(property, list[0], realm.CSS.px(4)), 'invalid typed list');
      throws(realm, () => map.append(property, 'linear-gradient()'), 'invalid appended image');
      check(element.style.getPropertyValue(property) === before, 'failed writes are atomic');
    }
    for (const property of ['border-image-source', 'list-style-image']) {
      throws(realm, () => map.set(property, ...list), 'single-valued property rejects list');
      throws(realm, () => map.append(property, list[0]), 'single-valued property rejects append');
    }
    for (const property of ['background', 'width', '--image']) {
      throws(realm, () => map.set(property, list[0]), 'image rejects incompatible property');
    }
    for (const source of ['linear-gradient()', 'image-set(url(a) -1x)', 'linear-gradient(red, blue); color: red']) {
      throws(realm, () => realm.CSSStyleValue.parse('background-image', source), 'image grammar remains authoritative');
    }
    check(realm.CSSStyleValue.parse('background', sources[0]).constructor === realm.CSSStyleValue, 'shorthand remains property-associated');
    check(realm.CSSStyleValue.parse('background-image', 'linear-gradient(var(--color), blue)') instanceof realm.CSSUnparsedValue, 'unresolved variables retain tokens');

    element.style.color = 'rgb(10, 20, 30)';
    element.style.fontSize = '10px';
    element.style.setProperty('--image', 'linear-gradient(currentcolor 1em, blue 50%)');
    element.style.backgroundImage = 'var(--image)';
    check(map.get('background-image') instanceof realm.CSSUnparsedValue, 'declared variable remains unparsed');
    const snapshot = element.computedStyleMap().get('background-image');
    const previous = String(snapshot);
    check(snapshot instanceof realm.CSSImageValue && previous.includes('10px'), 'computed image resolves relative lengths');
    // Typed OM exposes computed values. CSS Color 4 keeps currentcolor
    // symbolic until used-value time, unlike getComputedStyle's resolution.
    check(previous === 'linear-gradient(currentcolor 10px, rgb(0, 0, 255) 50%)', 'computed variable image retains currentcolor');
    const receiver = realm.document.createElement('div');
    receiver.style.color = 'rgb(40, 50, 60)';
    realm.document.body.append(receiver);
    receiver.attributeStyleMap.set('background-image', snapshot);
    check(String(receiver.attributeStyleMap.get('background-image')) === previous, 'computed image transfers its symbolic color');
    check(realm.getComputedStyle(receiver).backgroundImage === 'linear-gradient(rgb(40, 50, 60) 10px, rgb(0, 0, 255) 50%)', 'transferred image resolves against receiver color');
    receiver.remove();
    element.style.fontSize = '20px';
    check(String(element.computedStyleMap().get('background-image')).includes('20px'), 'computed image invalidates after style change');
    check(String(snapshot) === previous, 'image is an immutable snapshot');

    const value = list[0], text = String(value);
    Object.defineProperty(value, 'toString', {value() { throw new Error('author stringifier'); }});
    Object.defineProperty(value, Symbol.toPrimitive, {value() { throw new Error('author conversion'); }});
    Object.setPrototypeOf(value, null);
    map.set('list-style-image', value);
    check(String(map.get('list-style-image')) === text, 'native identity and serialization survive author changes');
    const sheet = new realm.CSSStyleSheet();
    sheet.replaceSync('div { background-image: ' + parts.join(', ') + '; }');
    const rule = sheet.cssRules[0];
    verifyList(rule.styleMap.getAll('background-image'), 'rule map');
    rule.styleMap.set('mask-image', ...list);
    verifyList(rule.styleMap.getAll('mask-image'), 'rule typed write');
    const retained = rule.styleMap.get('background-image');
    sheet.deleteRule(0);
    check(String(retained) === text, 'native image outlives removed rule');
    const detached = realm.document.implementation.createHTMLDocument('').createElement('div');
    detached.style.backgroundImage = parts.join(', ');
    verifyList(detached.attributeStyleMap.getAll('background-image'), 'windowless image list');
    element.remove();
  }
  const value = child.CSSStyleValue.parse('background-image', sources[6]);
  const element = child.document.createElement('div');
  child.document.body.append(element);
  StylePropertyMap.prototype.set.call(element.attributeStyleMap, 'list-style-image', value);
  const read = StylePropertyMapReadOnly.prototype.get.call(element.attributeStyleMap, 'list-style-image');
  check(read instanceof child.CSSImageValue && !(read instanceof CSSImageValue), 'cross-realm map uses owner realm');
  check(CSSStyleValue.prototype.toString.call(value) === String(read), 'cross-realm genuine image receiver');
  element.remove();
  return true;
})()
