JSON.stringify((() => {
  const failures = [], cases = [];
  let checks = 0;
  const property = 'font-style';
  const check = (ok, label) => { checks++; if (!ok) failures.push(label); };
  const attempt = (label, fn) => { try { fn(); } catch (e) { failures.push(label + ': ' + e); } };
  const keyword = (values, expected, label) => check(values.length === 1 &&
    values[0] instanceof CSSKeywordValue && values[0].value === expected && String(values[0]) === expected,
    label + ': ' + values.map(v => Object.prototype.toString.call(v) + ' ' + v));
  const opaque = (values, label) => check(values.length === 1 &&
    Object.prototype.toString.call(values[0]) === '[object CSSStyleValue]', label + ': ' + values.map(String));
  const throws = (fn, label) => {
    let error; try { fn(); } catch (e) { error = e; }
    check(error instanceof TypeError, label + ': ' + error);
  };
  const connected = document.createElement('div'); document.body.append(connected);
  const detached = document.createElement('div');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.append(svg);
  const owner = document.createElement('style'); owner.textContent = '.typed-font-style {}'; document.head.append(owner);
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.typed-font-style {}');
  const owners = [
    ['connected', connected.style, connected.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  for (const [source, expected] of [
    ['normal', 'normal'], ['NORMAL', 'normal'], ['n\\6f rmal', 'normal'],
    ['italic', 'italic'], ['ITALIC', 'italic'], ['i\\74 alic', 'italic'],
    ['oblique', 'oblique'], ['OBLIQUE', 'oblique'], ['o\\62lique', 'oblique'],
    ['oblique 0deg', 'normal'], ['oblique 14deg', 'oblique']
  ]) {
    attempt(source + ': parse', () => {
      keyword([CSSStyleValue.parse(property, source)], expected, source + ': parse');
      keyword(CSSStyleValue.parseAll(property, source), expected, source + ': parseAll');
    });
    for (const [kind, style, map] of owners) for (const write of ['string', 'keyword', 'parsed', 'cssom', 'cssText']) {
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
        check(style.getPropertyValue(property) === expected, label + ': canonical CSSOM');
        cases.push(label);
      });
    }
    attempt(source + ': computed', () => {
      connected.style.cssText = property + ': ' + source;
      keyword(connected.computedStyleMap().getAll(property), expected, source + ': computed keyword');
    });
  }
  for (const [source, computedKeyword] of [
    ['oblique 23deg', null], ['oblique -10deg', null],
    ['oblique -90deg', null], ['oblique 90deg', null],
    ['oblique calc(7deg + 7deg)', 'oblique'], ['oblique calc(0deg)', 'normal'],
    ['oblique calc(200deg)', null]
  ]) {
    attempt(source + ': parse', () => opaque(CSSStyleValue.parseAll(property, source), source + ': opaque parse'));
    for (const [kind, style, map] of owners) attempt(source + ': ' + kind, () => {
      style.cssText = 'color: red';
      map.set(property, CSSStyleValue.parse(property, source));
      opaque(map.getAll(property), source + ': declared angle');
      const text = style.getPropertyValue(property);
      check(text.startsWith('oblique '), source + ': angle retained');
      if (source.includes('calc(')) check(text.includes('calc('), source + ': specified calculation retained');
      style.setProperty('opacity', '0.25');
      opaque(map.getAll(property), source + ': opaque after unrelated write');
      check(String(map.get(property)) === text, source + ': full angle value retained');
      const before = style.cssText;
      throws(() => map.set(property, new CSSKeywordValue(source)), source + ': compound keyword rejected');
      check(style.cssText === before, source + ': atomic rejection');
      map.set(property, new CSSKeywordValue('oblique'));
      keyword(map.getAll(property), 'oblique', source + ': restore keyword');
    });
    attempt(source + ': computed', () => {
      connected.style.cssText = property + ': ' + source;
      if (computedKeyword === null) opaque(connected.computedStyleMap().getAll(property), source + ': computed angle');
      else keyword(connected.computedStyleMap().getAll(property), computedKeyword, source + ': resolved canonical keyword');
    });
  }
  for (const [kind, style, map] of owners) attempt(kind + ': validation', () => {
    map.set(property, 'oblique'); const saved = map.get(property);
    for (const invalid of ['none', 'normal italic', 'oblique oblique', 'oblique 91deg', 'oblique -91deg', 'oblique 1px', 'oblique 14deg 15deg']) {
      const before = style.cssText;
      throws(() => map.set(property, invalid), kind + ': invalid ' + invalid);
      check(style.cssText === before, kind + ': invalid write atomic');
    }
    throws(() => map.append(property, new CSSKeywordValue('normal')), kind + ': cannot append');
    throws(() => map.set(property, CSS.deg(23)), kind + ': bare angle rejected');
    map.set(property, new CSSUnparsedValue(['var(--style)']));
    check(map.get(property) instanceof CSSUnparsedValue, kind + ': variable stays unparsed');
    map.set(property, new CSSKeywordValue('inherit'));
    keyword(map.getAll(property), 'inherit', kind + ': CSS-wide keyword');
    keyword([saved], 'oblique', kind + ': immutable snapshot');
    map.set(property, saved); keyword(map.getAll(property), 'oblique', kind + ': restore snapshot');
  });
  attempt('inheritance and variables', () => {
    const child = document.createElement('span'); connected.append(child);
    connected.style.cssText = property + ': oblique';
    keyword(child.computedStyleMap().getAll(property), 'oblique', 'inherited oblique');
    connected.style.setProperty(property, 'italic');
    keyword(child.computedStyleMap().getAll(property), 'italic', 'inherited invalidation');
    child.style.cssText = '--style: oblique; ' + property + ': var(--style)';
    check(child.attributeStyleMap.get(property) instanceof CSSUnparsedValue, 'declared variable');
    keyword(child.computedStyleMap().getAll(property), 'oblique', 'computed variable');
    child.style.setProperty('--style', 'oblique 23deg');
    opaque(child.computedStyleMap().getAll(property), 'computed angular variable');
    child.style.setProperty('--style', 'normal');
    keyword(child.computedStyleMap().getAll(property), 'normal', 'variable invalidation');
    child.remove();
  });
  owner.remove(); connected.remove(); svg.remove();
  return {checks, cases, failures};
})())
