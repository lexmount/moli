JSON.stringify((() => {
  const failures = [];
  const cases = [];
  let checks = 0;
  const check = (condition, label) => {
    checks++;
    if (!condition) failures.push(label);
  };
  const brand = value => Object.prototype.toString.call(value);
  const keyword = (values, expected, label) => check(values.length === 1 &&
    values[0] instanceof CSSKeywordValue && values[0].value === expected &&
    values[0].toString() === expected, label + ': ' + values.map(v => brand(v) + ' ' + v));
  const opaque = (values, expected, label) => check(values.length === 1 &&
    brand(values[0]) === '[object CSSStyleValue]' &&
    (expected === null || values[0].toString() === expected), label + ': ' + values.map(v => brand(v) + ' ' + v));
  const throws = (callback, label) => {
    let error;
    try { callback(); } catch (e) { error = e; }
    check(error instanceof TypeError, label + ': ' + error);
  };
  const properties = [
    ['counter-increment', 'none', 'chapter 3'],
    ['counter-reset', 'none', 'chapter 3'],
    ['counter-set', 'none', 'chapter 3'],
    ['quotes', 'none', '"a" "b"'],
    ['font-feature-settings', 'normal', '"kern" 1'],
    ['font-variation-settings', 'normal', '"wght" 400'],
    ['font-variant-alternates', 'normal', 'swash(foo)'],
    ['font-language-override', 'normal', '"ENG"'],
    ['transition-property', 'none', 'width']
  ];
  const connected = document.createElement('div'); document.body.append(connected);
  const detached = document.createElement('div');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.append(svg);
  const owner = document.createElement('style'); owner.textContent = '.native-keyword {}'; document.head.append(owner);
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.native-keyword {}');
  const owners = [
    ['connected', connected.style, connected.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  for (const [property, expected, unsupported] of properties) {
    for (const source of [expected, expected.toUpperCase(), '\\6e ' + expected.slice(1)]) {
      try {
        keyword(CSSStyleValue.parseAll(property, source), expected, property + ': parseAll ' + source);
        keyword([CSSStyleValue.parse(property, source)], expected, property + ': parse ' + source);
      } catch (e) { failures.push(property + ': parse ' + source + ': ' + e); }
      for (const [kind, style, map] of owners) {
        for (const write of ['string', 'keyword', 'parsed', 'cssom', 'cssText']) {
          const label = [property, source, kind, write].join(': ');
          try {
            style.cssText = 'color: red';
            if (write === 'string') map.set(property, source);
            if (write === 'keyword') map.set(property, new CSSKeywordValue(source === expected.toUpperCase() ? source : expected));
            if (write === 'parsed') map.set(property, CSSStyleValue.parse(property, source));
            if (write === 'cssom') style.setProperty(property, source);
            if (write === 'cssText') style.cssText = property + ': ' + source;
            keyword(map.getAll(property), expected, label + ': getAll');
            keyword([map.get(property)], expected, label + ': get');
            keyword(Array.from(map).find(([name]) => name === property)[1], expected, label + ': iteration');
            style.setProperty('color', 'blue');
            keyword(map.getAll(property), expected, label + ': unrelated write');
            check(style.getPropertyValue(property) === expected, label + ': CSSOM serialization');
            cases.push({label, keyword: map.get(property).value});
          } catch (e) { failures.push(label + ': ' + e); }
        }
      }
    }
    for (const [kind, style, map] of owners) {
      const label = property + ': ' + kind;
      try {
        const parsed = CSSStyleValue.parse(property, unsupported);
        opaque([parsed], null, label + ': unsupported parse');
        map.set(property, parsed);
        opaque(map.getAll(property), style.getPropertyValue(property), label + ': unsupported map');
        style.setProperty('opacity', '0.5');
        opaque(map.getAll(property), style.getPropertyValue(property), label + ': unsupported after mutation');
        const before = style.cssText;
        throws(() => map.set(property, new CSSKeywordValue(unsupported)), label + ': reject compound keyword');
        check(style.cssText === before, label + ': invalid write is atomic');
        map.set(property, new CSSUnparsedValue(['var(--keyword)']));
        check(brand(map.get(property)) === '[object CSSUnparsedValue]', label + ': unparsed remains unparsed');
        map.set(property, new CSSKeywordValue('inherit'));
        keyword(map.getAll(property), 'inherit', label + ': CSS-wide keyword');
      } catch (e) { failures.push(label + ': ' + e); }
    }
    try {
      connected.style.cssText = property + ': ' + expected;
      keyword(connected.computedStyleMap().getAll(property), expected, property + ': computed keyword');
      connected.style.cssText = property + ': ' + unsupported;
      if (property === 'transition-property') keyword(connected.computedStyleMap().getAll(property), 'width', property + ': computed property name');
      else opaque(connected.computedStyleMap().getAll(property), null, property + ': computed unsupported');
    } catch (e) { failures.push(property + ': computed: ' + e); }
  }
  for (const [source, name] of [
    ['none', 'none'], ['all', 'all'], ['width', 'width'], ['WIDTH', 'width'],
    ['foo', 'foo'], ['--my-prop', '--my-prop'],
    ['foo\\2e bar', 'foo.bar'], ['--foo\\2e bar', '--foo.bar']
  ]) {
    try {
      connected.style.cssText = 'transition-property: ' + source;
      const values = connected.computedStyleMap().getAll('transition-property');
      check(values.length === 1 && values[0] instanceof CSSKeywordValue && values[0].value === name &&
        String(values[0]) === connected.style.getPropertyValue('transition-property'), source + ': computed identifier value and serialization');
    } catch (e) { failures.push(source + ': computed identifier: ' + e); }
  }
  for (const source of ['width, height', 'foo, bar', '--a, --b', 'width, foo']) {
    try {
      connected.style.cssText = 'transition-property: ' + source;
      opaque(connected.computedStyleMap().getAll('transition-property'), connected.style.getPropertyValue('transition-property'), source + ': computed opaque list');
    } catch (e) { failures.push(source + ': computed list: ' + e); }
  }
  try {
    connected.attributeStyleMap.set('counter-reset', 'none');
    throws(() => connected.attributeStyleMap.set('counter-reset', CSSStyleValue.parse('counter-increment', 'chapter 3')), 'opaque association across valid counter properties');
    connected.attributeStyleMap.set('transition-property', 'none');
    throws(() => connected.attributeStyleMap.append('transition-property', CSSStyleValue.parse('transition-property', 'width')), 'none cannot become a transition list');
    keyword(connected.attributeStyleMap.getAll('transition-property'), 'none', 'invalid append leaves none intact');
    keyword(CSSStyleValue.parseAll('transform', 'none'), 'none', 'transform none control');
    keyword(CSSStyleValue.parseAll('container-name', 'none'), 'none', 'container none control');
    keyword(CSSStyleValue.parseAll('quotes', 'auto'), 'auto', 'quotes auto control');
  } catch (e) { failures.push('boundaries: ' + e); }
  owner.remove(); connected.remove(); svg.remove();
  return {checks, cases, failures};
})())
