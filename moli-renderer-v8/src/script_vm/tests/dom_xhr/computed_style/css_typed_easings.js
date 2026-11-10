JSON.stringify((() => {
  const failures = [];
  const cases = [];
  let checks = 0;
  const check = (condition, label) => {
    checks++;
    if (!condition) failures.push(label);
  };
  const snapshot = values => values.map(value => [
    value instanceof CSSKeywordValue ? 'keyword' :
      Object.prototype.toString.call(value) === '[object CSSStyleValue]' ? 'function' : 'wrong',
    value.toString()
  ]);
  const same = (actual, expected, label) => check(actual.length === expected.length &&
    expected.every(([type, text], index) => actual[index][0] === type && (text === null || actual[index][1] === text)),
    label + ': ' + JSON.stringify(actual));
  const throws = (callback, label) => {
    let error;
    try { callback(); } catch (e) { error = e; }
    check(error instanceof TypeError, label);
  };
  const element = document.createElement('div');
  document.body.append(element);
  const detached = document.createElement('div');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  document.body.append(svg);
  const owner = document.createElement('style');
  owner.textContent = '.easing-rule {}';
  document.head.append(owner);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.easing-constructed {}');
  const owners = [
    ['inline', element.style, element.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  const syntaxes = [
    ['step-start', [['keyword', 'step-start']]],
    ['step-end', [['keyword', 'step-end']]],
    ['STEP-START', [['keyword', 'step-start']]],
    ['\\73 tep-end', [['keyword', 'step-end']]],
    ['steps(1, start)', [['function', 'steps(1, start)']]],
    ['steps(1, end)', [['function', 'steps(1)']]],
    ['step-start, ease', [['keyword', 'step-start'], ['keyword', 'ease']]],
    ['step-start, steps(4, end), step-end', [['function', null]]]
  ];
  for (const property of ['animation-timing-function', 'transition-timing-function']) {
    const initialOwner = document.createElement('style');
    initialOwner.textContent = '.initial-easing {' + property + ': step-start; color: red}';
    document.head.append(initialOwner);
    const initialSheet = new CSSStyleSheet();
    initialSheet.replaceSync('.initial-easing {' + property + ': step-start; color: red}');
    const initialMarkup = document.createElement('div');
    initialMarkup.innerHTML = '<div style="' + property + ': step-start; color: red"></div>';
    document.body.append(initialMarkup);
    for (const [name, style, map] of [
      ['parsed-style-rule', initialOwner.sheet.cssRules[0].style, initialOwner.sheet.cssRules[0].styleMap],
      ['parsed-constructed-rule', initialSheet.cssRules[0].style, initialSheet.cssRules[0].styleMap],
      ['parsed-inline', initialMarkup.firstChild.style, initialMarkup.firstChild.attributeStyleMap]
    ]) {
      same(snapshot(map.getAll(property)), [['keyword', 'step-start']], property + ': ' + name);
      style.setProperty('color', 'blue');
      same(snapshot(map.getAll(property)), [['keyword', 'step-start']], property + ': ' + name + ': unrelated write');
    }
    initialOwner.remove(); initialMarkup.remove();
  }
  for (const property of ['animation-timing-function', 'transition-timing-function']) {
    for (const [source, expected] of syntaxes) {
      // Unsupported parse() values can retain author text. Their native CSSOM
      // serialization is tested independently; keyword spelling is exact here.
      const parsedExpected = expected.map(([type, text]) => [type, type === 'function' ? null : text]);
      same(snapshot(CSSStyleValue.parseAll(property, source)), parsedExpected, property + ': parseAll ' + source);
      for (const [kind, style, map] of owners) {
        for (const write of ['string', 'parsed', 'cssom', 'cssText']) {
          const label = [property, source, kind, write].join(': ');
          try {
            style.cssText = 'color: red';
            if (write === 'string') map.set(property, source);
            if (write === 'parsed') map.set(property, ...CSSStyleValue.parseAll(property, source));
            if (write === 'cssom') style.setProperty(property, source);
            if (write === 'cssText') style.cssText = property + ': ' + source;
            const declaredExpected = expected.map(([type, text]) => [type, text === null ? style.getPropertyValue(property) : text]);
            same(snapshot(map.getAll(property)), declaredExpected, label + ': getAll');
            same(snapshot([map.get(property)]), declaredExpected.slice(0, 1), label + ': get');
            const entry = Array.from(map).find(([name]) => name === property);
            same(snapshot(entry[1]), declaredExpected, label + ': iteration');
            style.setProperty('opacity', '0.5');
            same(snapshot(map.getAll(property)), declaredExpected, label + ': unrelated mutation');
            cases.push({label, values: snapshot(map.getAll(property)), cssom: style.getPropertyValue(property)});
          } catch (e) { failures.push(label + ': ' + e.name + ': ' + e.message); }
        }
      }
    }
    for (const [kind, style, map] of owners) {
      const label = property + ': ' + kind;
      try {
        map.set(property, new CSSKeywordValue('step-start'));
        map.append(property, CSSStyleValue.parse(property, 'steps(4, end)'));
        map.append(property, new CSSKeywordValue('step-end'));
        same(snapshot(map.getAll(property)), [['function', style.getPropertyValue(property)]], label + ': append');
        const before = style.cssText;
        throws(() => map.set(property, new CSSKeywordValue('step-middle')), label + ': invalid keyword');
        check(style.cssText === before, label + ': invalid write atomicity');
        throws(() => map.set(property, new CSSKeywordValue('steps(1, start)')), label + ': function is not a keyword');
        check(style.cssText === before, label + ': invalid type atomicity');
        const other = property === 'animation-timing-function' ? 'transition-timing-function' : 'animation-timing-function';
        throws(() => map.set(other, CSSStyleValue.parse(property, 'steps(4)')), label + ': opaque association');
      } catch (e) { failures.push(label + ': ' + e.name + ': ' + e.message); }
    }
    for (const [source, expected] of syntaxes) {
      element.style.setProperty(property, source);
      const computed = element.computedStyleMap().getAll(property);
      const normalized = expected.map(([type, text]) => text === 'step-start' ? ['function', 'steps(1, start)'] :
        text === 'step-end' ? ['function', 'steps(1)'] : [type, text]);
      const computedExpected = normalized.some(([type]) => type === 'function') ?
        [['function', getComputedStyle(element).getPropertyValue(property)]] : normalized;
      same(snapshot(computed), computedExpected, property + ': computed ' + source);
    }
  }
  document.body.append(detached);
  check(detached.attributeStyleMap === owners[1][2], 'map identity after connecting');
  owner.remove(); element.remove(); detached.remove(); svg.remove();
  return {checks, cases, failures};
})())
