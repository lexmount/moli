(() => {
  'use strict';
  const check = (ok, label) => { if (!ok) throw new Error(label); };
  const equal = (actual, expected, label) => check(Object.is(actual, expected), label);
  const raises = (callback, Type = TypeError) => {
    try { callback(); } catch (error) { check(error instanceof Type, 'exception realm'); return; }
    throw new Error('missing exception');
  };
  const keyword = (value, expected, realm = globalThis) => {
    equal(Object.getPrototypeOf(value), realm.CSSKeywordValue.prototype, 'keyword realm');
    equal(value.value, expected, 'keyword value');
    equal(String(value), expected, 'keyword serialization');
  };
  const other = document.getElementById('child').contentWindow;
  const keywords = ['initial', 'inherit', 'unset', 'revert', 'revert-layer'];
  for (const realm of [globalThis, other]) {
    const parse = realm.CSSStyleValue.parse;
    for (const value of keywords) {
      keyword(parse('AlL', `/**/${value.toUpperCase()} /**/`), value, realm);
      const list = realm.CSSStyleValue.parseAll('all', value);
      equal(Object.getPrototypeOf(list), realm.Array.prototype, 'parseAll realm');
      equal(list.length, 1, 'all is single-valued');
      keyword(list[0], value, realm);
      for (const property of ['margin', 'background', 'border', 'font']) {
        equal(Object.getPrototypeOf(parse(property, value)), realm.CSSStyleValue.prototype,
          'ordinary shorthand remains property-associated');
      }
      equal(Object.getPrototypeOf(parse('--x', value)), realm.CSSUnparsedValue.prototype,
        'unregistered custom property remains unparsed');
    }
    for (const source of ['auto', 'initial inherit', 'initial, inherit', 'initial; color:red']) {
      raises(() => parse('all', source), realm.TypeError);
    }
    const unparsed = parse('all', 'var(--x, inherit)');
    check(unparsed instanceof realm.CSSUnparsedValue, 'variable reference stays unparsed');
    equal(unparsed[0].variable, '--x', 'native variable reference');
    const Constructor = realm.CSSKeywordValue;
    realm.CSSKeywordValue = () => { throw new Error('author constructor'); };
    try {
      equal(Object.getPrototypeOf(parse('all', 'inherit')), Constructor.prototype, 'intrinsic allocation');
    } finally { realm.CSSKeywordValue = Constructor; }
  }
  const elements = [
    document.createElement('div'),
    document.createElementNS('http://www.w3.org/2000/svg', 'g'),
    document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'),
    document.implementation.createHTMLDocument('').createElement('select'),
  ];
  const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
  const foreign = other.document.createElement('div');
  const foreignSheet = new other.CSSStyleSheet(); foreignSheet.replaceSync('div {}');
  const surfaces = [
    ...elements.map(element => [element.style, element.attributeStyleMap, globalThis]),
    [sheet.cssRules[0].style, sheet.cssRules[0].styleMap, globalThis],
    [foreign.style, foreign.attributeStyleMap, other],
    [foreignSheet.cssRules[0].style, foreignSheet.cssRules[0].styleMap, other],
  ];
  for (const [style, map, realm] of surfaces) {
    for (const value of keywords) {
      style.cssText = '';
      style.setProperty('all', value, 'important');
      const snapshot = StylePropertyMapReadOnly.prototype.get.call(map, 'all');
      keyword(snapshot, value, realm);
      equal(map.getAll('all').length, 1, 'one all value');
      keyword(map.getAll('all')[0], value, realm);
      snapshot.value = 'changed';
      keyword(map.get('all'), value, realm);
      const input = other.CSSStyleValue.parse('all', value);
      input.toString = () => { throw new Error('author stringifier'); };
      Object.defineProperty(input, 'value', {get() { throw new Error('author value getter'); }});
      StylePropertyMap.prototype.set.call(map, 'all', input);
      keyword(map.get('all'), value, realm);
      equal(style.getPropertyPriority('all'), '', 'typed write removes priority');
      const before = style.cssText;
      raises(() => map.set('all', CSS.px(1)), realm.TypeError);
      raises(() => map.set('all', 'initial', 'inherit'), realm.TypeError);
      raises(() => map.append('all', 'initial'), realm.TypeError);
      equal(style.cssText, before, 'invalid all write stays atomic');
      map.clear();
      StylePropertyMap.prototype.set.call(map, 'color', input);
      keyword(map.get('color'), value, realm);
      style.cssText = 'all: inherit';
      keyword(map.get('all'), 'inherit', realm);
      style.removeProperty('all');
      equal(map.getAll('all').length, 0, 'removed all declaration');
    }
  }
  const element = elements[0];
  const map = element.attributeStyleMap;
  document.body.append(element);
  element.setAttribute('style', 'all: initial');
  keyword(map.get('all'), 'initial');
  const observer = new MutationObserver(() => {});
  observer.observe(element, {attributes: true, attributeOldValue: true});
  map.set('all', CSSStyleValue.parse('all', 'inherit'));
  const records = observer.takeRecords(); observer.disconnect();
  equal(records.length, 1, 'one style mutation');
  equal(records[0].oldValue, 'all: initial', 'mutation old value');
  keyword(map.get('all'), 'inherit');
  element.remove();
  return true;
})()
