(() => {
  'use strict';
  const check = (ok, label) => { if (!ok) throw new Error(label); };
  const equal = (actual, expected, label) => check(Object.is(actual, expected), label);
  const raises = (callback, Type) => {
    try { callback(); } catch (error) { check(error instanceof Type, 'exception realm'); return; }
    throw new Error('missing exception');
  };
  const unit = (value, magnitude, name, realm = globalThis) => {
    equal(Object.getPrototypeOf(value), realm.CSSUnitValue.prototype, 'unit realm');
    equal(value.value, magnitude, 'magnitude');
    equal(value.unit, name, 'unit');
  };
  const sum = (value, expected, realm = globalThis) => {
    equal(Object.getPrototypeOf(value), realm.CSSMathSum.prototype, 'sum realm');
    equal(Object.getPrototypeOf(value.values), realm.CSSNumericArray.prototype, 'operand array realm');
    equal(value.values.length, expected.length, 'operand count');
    for (const [magnitude, name] of expected) {
      const found = Array.from(value.values).find(operand => operand.unit === name);
      check(found, 'missing ' + name);
      unit(found, magnitude, name, realm);
    }
  };
  const other = document.getElementById('child').contentWindow;
  for (const realm of [globalThis, other]) {
    const parse = realm.CSSStyleValue.parse;
    const mixed = parse('width', 'calc(0.1234567890123456px + 2em + 3%)');
    sum(mixed, [[0.1234567890123456, 'px'], [2, 'em'], [3, 'percent']], realm);
    unit(mixed.values[0], 0.1234567890123456, 'px', realm);
    const list = realm.CSSStyleValue.parseAll('transition-duration', 'calc(0.1234567890123456s + 1s), 2s');
    equal(list.length, 2, 'list cardinality');
    equal(list[0].to('s').value, 1.1234567890123457, 'math list source precision');
    equal(list[1].value, 2, 'second list item');
    for (const [text, Type] of [
      ['min(1em, 2px)', realm.CSSMathMin],
      ['max(1em, 2px)', realm.CSSMathMax],
    ]) {
      const value = parse('width', text);
      equal(Object.getPrototypeOf(value), Type.prototype, text);
      unit(value.values[0], 1, 'em', realm);
      unit(value.values[1], 2, 'px', realm);
    }
    const clamp = parse('width', 'clamp(1em, 2px, 3rem)');
    equal(Object.getPrototypeOf(clamp), realm.CSSMathClamp.prototype, 'clamp');
    unit(clamp.lower, 1, 'em', realm);
    unit(clamp.value, 2, 'px', realm);
    unit(clamp.upper, 3, 'rem', realm);
    raises(() => parse('width', 'calc(1px + 2s)'), realm.TypeError);
    raises(() => parse('color', 'calc(1px + 2em)'), realm.TypeError);
    equal(Object.getPrototypeOf(parse('color', 'red')), realm.CSSStyleValue.prototype, 'opaque color stays opaque');
    const sentinel = {};
    let conversions = 0;
    try {
      parse('width', {toString() {conversions++; throw sentinel;}});
      throw new Error('missing conversion exception');
    } catch (error) { equal(error, sentinel, 'conversion exception identity'); }
    equal(conversions, 1, 'convert once');
    const constructors = ['CSSUnitValue', 'CSSNumericArray', 'CSSMathSum'];
    const originals = constructors.map(name => realm[name]);
    const numericParse = realm.CSSNumericValue.parse;
    try {
      for (const name of constructors) realm[name] = () => { throw sentinel; };
      realm.CSSNumericValue.parse = () => { throw sentinel; };
      const value = parse('width', 'calc(1px + 2em)');
      equal(Object.getPrototypeOf(value), originals[2].prototype, 'intrinsic sum allocation');
      equal(Object.getPrototypeOf(value.values), originals[1].prototype, 'intrinsic array allocation');
      equal(Object.getPrototypeOf(value.values[0]), originals[0].prototype, 'intrinsic unit allocation');
    } finally {
      constructors.forEach((name, index) => { realm[name] = originals[index]; });
      realm.CSSNumericValue.parse = numericParse;
    }
  }
  const elements = [
    document.createElement('div'),
    document.createElementNS('http://www.w3.org/2000/svg', 'g'),
    document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'),
    document.implementation.createHTMLDocument('').createElement('select'),
  ];
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { width: 1px; }');
  const foreignSheet = new other.CSSStyleSheet();
  foreignSheet.replaceSync('div { width: 1px; }');
  const foreign = other.document.createElement('div');
  const surfaces = [
    ...elements.map(element => [element.style, element.attributeStyleMap, globalThis]),
    [sheet.cssRules[0].style, sheet.cssRules[0].styleMap, globalThis],
    [foreign.style, foreign.attributeStyleMap, other],
    [foreignSheet.cssRules[0].style, foreignSheet.cssRules[0].styleMap, other],
  ];
  for (const [style, map, realm] of surfaces) {
    style.setProperty('width', 'calc(1px + 2em + 3%)');
    const snapshot = StylePropertyMapReadOnly.prototype.get.call(map, 'width');
    sum(snapshot, [[1, 'px'], [2, 'em'], [3, 'percent']], realm);
    equal(map.getAll('width').length, 1, 'single calculation');
    sum(map.getAll('width')[0], [[1, 'px'], [2, 'em'], [3, 'percent']], realm);
    sum([...map].find(([name]) => name === 'width')[1][0], [[1, 'px'], [2, 'em'], [3, 'percent']], realm);
    snapshot.values[0].value = 99;
    sum(map.get('width'), [[1, 'px'], [2, 'em'], [3, 'percent']], realm);
    style.width = 'min(4em, 5px)';
    equal(Object.getPrototypeOf(map.get('width')), realm.CSSMathMin.prototype, 'fresh CSSOM mutation');
    const input = other.CSSStyleValue.parse('width', 'calc(7px + 8em)');
    input.toString = () => { throw new Error('author toString'); };
    Object.defineProperty(input, 'values', {get() { throw new Error('author values'); }});
    map.set('height', input);
    sum(map.get('height'), [[7, 'px'], [8, 'em']], realm);
    const before = style.cssText;
    raises(() => map.set('transition-duration', input), realm.TypeError);
    equal(style.cssText, before, 'invalid mathematical write stays atomic');
    style.cssText = 'width: max(9em, 10px)';
    equal(Object.getPrototypeOf(map.get('width')), realm.CSSMathMax.prototype, 'fresh cssText');
    style.removeProperty('width');
    equal(map.getAll('width').length, 0, 'removed declaration');
  }
  const element = elements[0];
  const retainedMap = element.attributeStyleMap;
  document.body.append(element);
  element.setAttribute('style', 'width: calc(11px + 12em)');
  sum(retainedMap.get('width'), [[11, 'px'], [12, 'em']]);
  element.remove();
  return true;
})()
