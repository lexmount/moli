JSON.stringify((() => {
  const failures = [], cases = [];
  let checks = 0;
  const property = 'font-palette';
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
  const owner = document.createElement('style'); owner.textContent = '.typed-font-palette {}'; document.head.append(owner);
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.typed-font-palette {}');
  const owners = [
    ['connected', connected.style, connected.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  for (const [source, expected] of [
    ['normal', 'normal'], ['NORMAL', 'normal'], ['n\\6f rmal', 'normal'],
    ['light', 'light'], ['LIGHT', 'light'], ['l\\69ght', 'light'],
    ['dark', 'dark'], ['DARK', 'dark'], ['d\\61rk', 'dark']
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
  for (const source of ['--foo', '--Fóo', '--normal', '--foo\\2e bar', '--foo\\20 bar']) {
    attempt(source + ': parse', () => opaque(CSSStyleValue.parseAll(property, source), source + ': named palette'));
    for (const [kind, style, map] of owners) attempt(source + ': ' + kind, () => {
      style.cssText = 'color: red';
      const parsed = CSSStyleValue.parse(property, source); map.set(property, parsed);
      opaque(map.getAll(property), source + ': declared named palette');
      const text = style.getPropertyValue(property);
      check(text.startsWith('--'), source + ': name retained');
      style.setProperty('opacity', '0.25');
      opaque(map.getAll(property), source + ': opaque after unrelated write');
      check(String(map.get(property)) === text, source + ': full name retained');
      const before = style.cssText;
      throws(() => map.set(property, new CSSKeywordValue(source)), source + ': named palette is not a supported keyword');
      check(style.cssText === before, source + ': atomic keyword rejection');
      throws(() => map.set('anchor-scope', parsed), source + ': opaque association across valid dashed-name properties');
      map.set(property, new CSSKeywordValue('dark'));
      keyword(map.getAll(property), 'dark', source + ': restore keyword');
    });
    attempt(source + ': computed', () => {
      connected.style.cssText = property + ': ' + source;
      opaque(connected.computedStyleMap().getAll(property), source + ': computed named palette');
    });
  }
  for (const [kind, style, map] of owners) attempt(kind + ': validation', () => {
    map.set(property, 'dark'); const saved = map.get(property);
    for (const invalid of ['Augusta', 'none', 'light dark', '--foo --bar', 'normal, dark', '"normal"']) {
      const before = style.cssText;
      throws(() => map.set(property, invalid), kind + ': invalid ' + invalid);
      check(style.cssText === before, kind + ': invalid write atomic');
    }
    throws(() => map.append(property, new CSSKeywordValue('light')), kind + ': cannot append');
    map.set(property, new CSSUnparsedValue(['var(--palette)']));
    check(map.get(property) instanceof CSSUnparsedValue, kind + ': variable stays unparsed');
    map.set(property, new CSSKeywordValue('inherit'));
    keyword(map.getAll(property), 'inherit', kind + ': CSS-wide keyword');
    keyword([saved], 'dark', kind + ': immutable snapshot');
    map.set(property, saved); keyword(map.getAll(property), 'dark', kind + ': restore snapshot');
  });
  attempt('inheritance and variables', () => {
    const child = document.createElement('span'); connected.append(child);
    connected.style.cssText = property + ': dark';
    keyword(child.computedStyleMap().getAll(property), 'dark', 'inherited dark');
    connected.style.setProperty(property, 'light');
    keyword(child.computedStyleMap().getAll(property), 'light', 'inherited invalidation');
    child.style.cssText = '--palette: dark; ' + property + ': var(--palette)';
    check(child.attributeStyleMap.get(property) instanceof CSSUnparsedValue, 'declared variable');
    keyword(child.computedStyleMap().getAll(property), 'dark', 'computed variable');
    child.style.setProperty('--palette', '--Fóo');
    opaque(child.computedStyleMap().getAll(property), 'computed named variable');
    child.style.setProperty('--palette', 'normal');
    keyword(child.computedStyleMap().getAll(property), 'normal', 'variable invalidation');
    child.remove();
  });
  owner.remove(); connected.remove(); svg.remove();
  return {checks, cases, failures};
})())
