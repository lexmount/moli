(() => {
  'use strict';
  const check = (ok, label) => { if (!ok) throw new Error(label); };
  const equal = (actual, expected, label) => check(Object.is(actual, expected), `${label}: ${actual} !== ${expected}`);
  const other = document.getElementById('child').contentWindow;
  const elements = [
    document.createElement('div'),
    document.createElementNS('http://www.w3.org/2000/svg', 'g'),
    document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'),
    document.implementation.createHTMLDocument('').createElement('select'),
  ];
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div {}');
  const foreignSheet = new other.CSSStyleSheet();
  foreignSheet.replaceSync('div {}');
  const foreign = other.document.createElement('div');
  const surfaces = [
    ...elements.map(element => [element.style, element.attributeStyleMap, globalThis]),
    [sheet.cssRules[0].style, sheet.cssRules[0].styleMap, globalThis],
    [foreign.style, foreign.attributeStyleMap, other],
    [foreignSheet.cssRules[0].style, foreignSheet.cssRules[0].styleMap, other],
  ];
  const fractional = () => new CSSMathSum(CSS.px(100000.4), CSS.px(-100000));
  for (const [style, map, realm] of surfaces) {
    const values = [
      new CSSMathSum(CSS.px(1000001), CSS.px(-1000000)),
      fractional(),
      new CSSMathProduct(fractional(), CSS.number(1.1234567890123456)),
      new CSSMathMin(fractional(), CSS.px(2)),
      new CSSMathMax(fractional(), CSS.px(0.1234567890123456)),
      new CSSMathClamp(CSS.px(0.1234567890123456), fractional(), CSS.px(2)),
      new CSSMathNegate(CSS.px(0.1234567890123456)),
      new CSSMathProduct(CSS.px(1.1234567890123456), new CSSMathInvert(CSS.number(3.141592653589793))),
    ];
    for (const [index, value] of values.entries()) {
      const before = value.to('px').value;
      const display = String(value);
      map.set('margin-left', value);
      equal(map.get('margin-left').to('px').value, before, `case ${index} round trip`);
      equal(map.getAll('margin-left')[0].to('px').value, before, `case ${index} getAll`);
      equal([...map].find(([name]) => name === 'margin-left')[1][0].to('px').value, before, `case ${index} iteration`);
      check(map.get('margin-left') instanceof realm.CSSMathValue, 'math value in the owner realm');
      equal(String(value), display, 'declaration writes do not change public stringification');
      style.color = 'red';
      equal(map.get('margin-left').to('px').value, before, 'unrelated CSSOM write retains numeric snapshot');
      style.setProperty('margin-left', 'invalid');
      equal(map.get('margin-left').to('px').value, before, 'invalid CSSOM write stays atomic');
    }
    const first = CSS.px(100000.4);
    const input = new CSSMathSum(first, CSS.px(-100000));
    const expected = input.to('px').value;
    input.toString = () => { throw new Error('author stringifier'); };
    Object.defineProperty(input, 'values', { get() { throw new Error('author operands'); } });
    map.set('margin-left', input);
    first.value = 0;
    equal(map.get('margin-left').to('px').value, expected, 'input mutations cannot change the declaration snapshot');
    const snapshot = map.get('margin-left');
    snapshot.values[0].value = 999;
    equal(map.get('margin-left').to('px').value, expected, 'read values are fresh snapshots');
    const cssom = style.marginLeft;
    style.marginLeft = cssom;
    equal(map.get('margin-left').to('px').value, CSSStyleValue.parse('margin-left', cssom).to('px').value, 'same-text CSSOM rewrite invalidates typed snapshot');
    map.set('margin-left', fractional());
    style.margin = '2px';
    equal(map.get('margin-left').to('px').value, 2, 'shorthand write invalidates affected snapshots');
    const duration = new CSSMathSum(CSS.s(100000.4), CSS.s(-100000));
    const last = new CSSMathSum(CSS.s(1000001), CSS.s(-1000000));
    map.set('transition-duration', duration, CSS.s(2.1234567890123456));
    map.append('transition-duration', last);
    const list = map.getAll('transition-duration');
    equal(list.length, 3, 'numeric list cardinality after append');
    equal(list[0].to('s').value, duration.to('s').value, 'append retains existing expression precision');
    equal(list[1].to('s').value, 2.1234567890123456, 'numeric list retains unit precision');
    equal(list[2].to('s').value, 1, 'append retains new expression precision');
    map.delete('transition-duration');
    equal(map.getAll('transition-duration').length, 0, 'deletion removes expression snapshot');
    map.set('margin-left', fractional());
    style.cssText = 'margin-left: 3px';
    equal(map.get('margin-left').to('px').value, 3, 'cssText replacement invalidates snapshots');
    map.clear();
    equal(map.size, 0, 'clear removes snapshots with declarations');
  }
  return true;
})()
