JSON.stringify((() => {
  const failures = [];
  const cases = [];
  let checks = 0;
  const property = 'font-variant-alternates';
  const check = (condition, label) => {
    checks++;
    if (!condition) failures.push(label);
  };
  const attempt = (label, callback) => {
    try { callback(); } catch (error) { failures.push(label + ': ' + error); }
  };
  const keyword = (values, expected, label) => check(values.length === 1 &&
    values[0] instanceof CSSKeywordValue && values[0].value === expected &&
    String(values[0]) === expected, label + ': keyword ' + values.map(String));
  const opaque = (values, label) => check(values.length === 1 &&
    Object.prototype.toString.call(values[0]) === '[object CSSStyleValue]', label + ': opaque ' + values.map(String));
  const throws = (callback, label) => {
    let error;
    try { callback(); } catch (caught) { error = caught; }
    check(error instanceof TypeError, label + ': ' + error);
  };
  const connected = document.createElement('div'); document.body.append(connected);
  const detached = document.createElement('div');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.append(svg);
  const owner = document.createElement('style'); owner.textContent = '.font-alternates {}'; document.head.append(owner);
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.font-alternates {}');
  const owners = [
    ['connected', connected.style, connected.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  for (const [source, expected] of [
    ['normal', 'normal'], ['NORMAL', 'normal'], ['n\\6f rmal', 'normal'],
    ['historical-forms', 'historical-forms'], ['HISTORICAL-FORMS', 'historical-forms'],
    ['h\\69storical-forms', 'historical-forms']
  ]) {
    attempt(source + ': parsing', () => {
      keyword([CSSStyleValue.parse(property, source)], expected, source + ': parse');
      keyword(CSSStyleValue.parseAll(property, source), expected, source + ': parseAll');
    });
    for (const [kind, style, map] of owners) {
      for (const write of ['string', 'keyword', 'parsed', 'cssom', 'cssText']) {
        const label = [source, kind, write].join(': ');
        attempt(label, () => {
          style.cssText = 'color: red';
          if (write === 'string') map.set(property, source);
          if (write === 'keyword') map.set(property, new CSSKeywordValue(source === source.toUpperCase() ? source : expected));
          if (write === 'parsed') map.set(property, CSSStyleValue.parse(property, source));
          if (write === 'cssom') style.setProperty(property, source);
          if (write === 'cssText') style.cssText = property + ': ' + source;
          keyword(map.getAll(property), expected, label + ': getAll');
          keyword([map.get(property)], expected, label + ': get');
          keyword(Array.from(map).find(([name]) => name === property)[1], expected, label + ': iteration');
          style.setProperty('opacity', '0.5');
          keyword(map.getAll(property), expected, label + ': unrelated write');
          check(style.getPropertyValue(property) === expected, label + ': CSSOM serialization');
          if (kind === 'style-rule' || kind === 'constructed-rule') {
            const rule = kind === 'style-rule' ? owner.sheet.cssRules[0] : sheet.cssRules[0];
            keyword(rule.styleMap.getAll(property), expected, label + ': reacquired rule');
          }
          cases.push(label);
        });
      }
    }
    attempt(source + ': computed', () => {
      connected.style.cssText = property + ': ' + source;
      keyword(connected.computedStyleMap().getAll(property), expected, source + ': computed');
    });
  }
  for (const source of [
    'stylistic(foo)', 'styleset(foo, bar)', 'character-variant(foo, bar)',
    'swash(foo)', 'ornaments(foo)', 'annotation(foo)',
    'historical-forms swash(foo)', 'swash(foo) historical-forms',
    'swash(foo) annotation(bar)', 'historical-forms styleset(foo, bar) annotation(baz)'
  ]) {
    attempt(source + ': unsupported parse', () => {
      opaque(CSSStyleValue.parseAll(property, source), source);
    });
    for (const [kind, style, map] of owners) {
      const label = source + ': ' + kind;
      attempt(label, () => {
        const parsed = CSSStyleValue.parse(property, source);
        style.cssText = 'color: red'; map.set(property, parsed);
        opaque(map.getAll(property), label + ': opaque');
        const text = style.getPropertyValue(property);
        check(text.includes('('), label + ': functions retained');
        style.setProperty('opacity', '0.25');
        opaque(map.getAll(property), label + ': after mutation');
        check(String(map.get(property)) === text, label + ': entire declaration retained');
        const before = style.cssText;
        throws(() => map.set(property, new CSSKeywordValue(source)), label + ': compound keyword rejected');
        check(style.cssText === before, label + ': atomic rejection');
        map.set(property, new CSSKeywordValue('historical-forms'));
        keyword(map.getAll(property), 'historical-forms', label + ': restore keyword');
      });
    }
    attempt(source + ': computed opaque', () => {
      connected.style.cssText = property + ': ' + source;
      opaque(connected.computedStyleMap().getAll(property), source + ': computed');
    });
  }
  for (const [kind, style, map] of owners) attempt(kind + ': validation', () => {
    map.set(property, 'historical-forms');
    const saved = map.get(property);
    for (const invalid of ['none', 'historical-forms historical-forms', 'normal historical-forms', 'historical-forms, normal', 'swash()']) {
      const before = style.cssText;
      throws(() => map.set(property, invalid), kind + ': invalid ' + invalid);
      check(style.cssText === before, kind + ': invalid write is atomic');
    }
    throws(() => map.append(property, new CSSKeywordValue('normal')), kind + ': cannot append');
    keyword([saved], 'historical-forms', kind + ': snapshot');
    map.set(property, new CSSUnparsedValue(['var(--alternates)']));
    check(map.get(property) instanceof CSSUnparsedValue, kind + ': variable stays unparsed');
    map.set(property, new CSSKeywordValue('inherit'));
    keyword(map.getAll(property), 'inherit', kind + ': CSS-wide keyword');
    map.set(property, saved);
    keyword(map.getAll(property), 'historical-forms', kind + ': saved value restored');
  });
  attempt('inheritance and variables', () => {
    const child = document.createElement('span'); connected.append(child);
    connected.style.cssText = property + ': historical-forms';
    keyword(child.computedStyleMap().getAll(property), 'historical-forms', 'inherited keyword');
    connected.style.setProperty(property, 'normal');
    keyword(child.computedStyleMap().getAll(property), 'normal', 'inherited keyword invalidation');
    child.style.cssText = '--alternates: historical-forms; ' + property + ': var(--alternates)';
    check(child.attributeStyleMap.get(property) instanceof CSSUnparsedValue, 'declared variable');
    keyword(child.computedStyleMap().getAll(property), 'historical-forms', 'computed variable');
    child.style.setProperty('--alternates', 'swash(foo)');
    opaque(child.computedStyleMap().getAll(property), 'computed functional variable');
    child.remove();
  });
  owner.remove(); connected.remove(); svg.remove();
  return {checks, cases, failures};
})())
