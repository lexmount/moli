(() => {
  const check = (ok, label) => { if (!ok) throw new Error(label); };
  const throws = (realm, run, label) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    check(error instanceof realm.TypeError, label);
  };
  const child = document.querySelector('iframe').contentWindow;
  const properties = ['background-image', 'border-image-source', 'list-style-image'];
  const source = 'url("/typed-image.png")';
  for (const realm of [window, child]) {
    const C = realm.CSSImageValue;
    check(typeof C === 'function' && C.name === 'CSSImageValue' && C.length === 0, 'interface object');
    check(Object.getPrototypeOf(C) === realm.CSSStyleValue, 'constructor inheritance');
    check(Object.getPrototypeOf(C.prototype) === realm.CSSStyleValue.prototype, 'prototype inheritance');
    check(!Object.hasOwn(C, 'parse') && !Object.hasOwn(C.prototype, 'toString'), 'inherited operations');
    const global = Object.getOwnPropertyDescriptor(realm, 'CSSImageValue');
    check(global.writable && global.configurable && !global.enumerable, 'global descriptor');
    throws(realm, () => C(), 'call illegal constructor');
    throws(realm, () => new C(), 'new illegal constructor');
    const value = realm.CSSStyleValue.parse('background-image', source);
    const stringify = realm.CSSStyleValue.prototype.toString;
    check(value instanceof C && value instanceof realm.CSSStyleValue, 'native inheritance');
    check(Object.prototype.toString.call(value) === '[object CSSImageValue]', 'class tag');
    check(Object.getPrototypeOf(value) === C.prototype && stringify.call(value) === source, 'native prototype and serialization');
    for (const text of ['url("")', 'url("http://[")', 'url("data:text/plain,not-an-image")']) {
      check(realm.CSSStyleValue.parse('background-image', text) instanceof C, 'CSS URL does not require a decodable resource');
    }
    const list = realm.CSSStyleValue.parseAll('background-image', source + ', none, url("/second.png")');
    check(list.length === 3 && list[0] instanceof C && list[1] instanceof realm.CSSKeywordValue && list[2] instanceof C, 'list projection');
    const element = realm.document.createElement('div');
    realm.document.body.append(element);
    const map = element.attributeStyleMap;
    let authorCalls = 0;
    Object.defineProperty(value, 'toString', {value() { authorCalls++; throw new Error('author stringifier'); }});
    Object.defineProperty(value, Symbol.toPrimitive, {value() { authorCalls++; throw new Error('author primitive'); }});
    for (const property of properties) {
      map.set(property, value);
      const declared = map.get(property);
      check(declared instanceof C && stringify.call(declared) === source, 'cross-property native image');
      check(element.style.getPropertyValue(property) === source, 'declaration serialization');
      const computed = element.computedStyleMap().get(property);
      check(computed instanceof C && stringify.call(computed) === 'url("' + new URL('/typed-image.png', realm.document.baseURI).href + '")', 'computed URL projection');
    }
    check(authorCalls === 0, 'native write bypasses author conversion');
    for (const property of ['width', 'background', '--image']) {
      throws(realm, () => map.set(property, value), 'image rejected for incompatible property');
    }
    const before = element.style.backgroundImage;
    throws(realm, () => map.set('background-image', value, realm.CSS.px(4)), 'atomic invalid list');
    check(element.style.backgroundImage === before, 'invalid list preserves declaration');
    map.set('background-image', value);
    map.append('background-image', realm.CSSStyleValue.parse('list-style-image', 'url("/second.png")'));
    check(map.getAll('background-image').length === 2 && map.getAll('background-image').every(item => item instanceof C), 'native image append');
    map.set('background-image', 'none');
    check(stringify.call(value) === source && map.get('background-image') instanceof realm.CSSKeywordValue, 'values snapshot declarations');
    let traps = 0;
    const proxy = new Proxy(value, {get() { traps++; throw new Error('get trap'); }, getPrototypeOf() { traps++; throw new Error('prototype trap'); }});
    const revoked = Proxy.revocable(value, {}); revoked.revoke();
    for (const invalid of [{}, C.prototype, Object.create(value), Object.create(C.prototype), proxy, revoked.proxy, null, undefined, 3]) {
      throws(realm, () => stringify.call(invalid), 'stringifier requires a native receiver');
    }
    check(traps === 0, 'brand check does not observe proxy traps');
    Object.setPrototypeOf(value, null);
    check(stringify.call(value) === source, 'native identity survives prototype changes');
    map.set('list-style-image', value);
    check(authorCalls === 0 && map.get('list-style-image') instanceof C, 'native write survives prototype changes');
    const sheet = new realm.CSSStyleSheet();
    sheet.replaceSync('div { background-image: ' + source + '; }');
    const rule = sheet.cssRules[0];
    const retained = rule.styleMap.get('background-image');
    check(retained instanceof C, 'rule style map projection');
    sheet.deleteRule(0);
    check(String(retained) === source, 'value survives rule removal');
    const detached = realm.document.implementation.createHTMLDocument('').createElement('div');
    detached.style.backgroundImage = source;
    check(detached.attributeStyleMap.get('background-image') instanceof C, 'windowless declaration projection');
    realm.CSSImageValue = function() { throw new Error('author constructor'); };
    check(realm.CSSStyleValue.parse('background-image', source) instanceof C && map.get('list-style-image') instanceof C, 'intrinsic allocation');
    Object.defineProperty(realm, 'CSSImageValue', global);
    element.remove();
  }
  const value = child.CSSStyleValue.parse('background-image', source);
  check(CSSStyleValue.prototype.toString.call(value) === source, 'genuine cross-realm stringifier');
  const element = child.document.createElement('div');
  child.document.body.append(element);
  StylePropertyMap.prototype.set.call(element.attributeStyleMap, 'list-style-image', value);
  const read = StylePropertyMapReadOnly.prototype.get.call(element.attributeStyleMap, 'list-style-image');
  check(read instanceof child.CSSImageValue && !(read instanceof CSSImageValue), 'map owner realm');
  const parsed = CSSStyleValue.parse.call(child.CSSImageValue, 'background-image', source);
  check(parsed instanceof CSSImageValue && !(parsed instanceof child.CSSImageValue), 'static parser callee realm');
  element.remove();
  return true;
})()
