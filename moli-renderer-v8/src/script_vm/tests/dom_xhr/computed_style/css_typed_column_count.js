JSON.stringify((() => {
  const failures = [], cases = [];
  let checks = 0;
  const property = 'column-count';
  const check = (ok, label) => { checks++; if (!ok) failures.push(label); };
  const attempt = (label, fn) => { try { fn(); } catch (e) { failures.push(label + ': ' + e); } };
  const numeric = (values, expected, label) => check(values.length === 1 &&
    values[0] instanceof CSSNumericValue && Math.abs(values[0].to('number').value - expected) < 0.000001,
    label + ': numeric ' + values.map(String));
  const keyword = (values, expected, label) => check(values.length === 1 &&
    values[0] instanceof CSSKeywordValue && values[0].value === expected, label + ': keyword');
  const throws = (fn, label) => {
    let error; try { fn(); } catch (e) { error = e; }
    check(error instanceof TypeError, label + ': ' + error);
  };
  const connected = document.createElement('div'); document.body.append(connected);
  const detached = document.createElement('div');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.append(svg);
  const owner = document.createElement('style'); owner.textContent = '.column-count-values {}'; document.head.append(owner);
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.column-count-values {}');
  const owners = [
    ['connected', connected.style, connected.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  for (const [source, expected, computed] of [
    ['1', 1, 1], ['2', 2, 2], ['12', 12, 12], ['calc(0)', 0, 1],
    ['calc(-3.14)', -3.14, 1], ['calc(3.14)', 3.14, 3], ['calc(2 + 3)', 5, 5],
    ['min(2, 3)', 2, 2], ['max(2, 3)', 3, 3], ['clamp(1, 3, 7)', 3, 3]
  ]) {
    attempt(source + ': parsing', () => {
      numeric([CSSStyleValue.parse(property, source)], expected, source + ': parse');
      numeric(CSSStyleValue.parseAll(property, source), expected, source + ': parseAll');
    });
    for (const [kind, style, map] of owners) for (const write of ['string', 'unit', 'parsed', 'cssom', 'cssText']) {
      const label = [source, kind, write].join(': ');
      attempt(label, () => {
        style.cssText = 'color: red';
        if (write === 'string') map.set(property, source);
        if (write === 'unit') map.set(property, CSS.number(expected));
        if (write === 'parsed') map.set(property, CSSStyleValue.parse(property, source));
        if (write === 'cssom') style.setProperty(property, source);
        if (write === 'cssText') style.cssText = property + ': ' + source;
        numeric(map.getAll(property), expected, label + ': getAll');
        numeric([map.get(property)], expected, label + ': get');
        numeric(Array.from(map).find(([name]) => name === property)[1], expected, label + ': iteration');
        style.setProperty('opacity', '0.5');
        numeric(map.getAll(property), expected, label + ': unrelated write');
        if (kind.endsWith('rule')) {
          const rule = kind === 'style-rule' ? owner.sheet.cssRules[0] : sheet.cssRules[0];
          numeric(rule.styleMap.getAll(property), expected, label + ': reacquired rule');
        }
        cases.push(label);
      });
    }
    attempt(source + ': computed', () => {
      connected.style.cssText = property + ': ' + source;
      numeric(connected.computedStyleMap().getAll(property), computed, source + ': computed positive integer');
    });
  }
  for (const source of ['auto', 'AUTO', '\\61 uto']) {
    keyword(CSSStyleValue.parseAll(property, source), 'auto', source + ': parse keyword');
    for (const [kind, style, map] of owners) for (const write of ['string', 'keyword', 'parsed', 'cssom', 'cssText']) {
      const label = [source, kind, write].join(': ');
      attempt(label, () => {
        style.cssText = '';
        if (write === 'string') map.set(property, source);
        if (write === 'keyword') map.set(property, new CSSKeywordValue(source === 'AUTO' ? source : 'auto'));
        if (write === 'parsed') map.set(property, CSSStyleValue.parse(property, source));
        if (write === 'cssom') style.setProperty(property, source);
        if (write === 'cssText') style.cssText = property + ': ' + source;
        keyword(map.getAll(property), 'auto', label);
        cases.push(label);
      });
    }
    connected.style.cssText = property + ': ' + source;
    keyword(connected.computedStyleMap().getAll(property), 'auto', source + ': computed auto');
  }
  for (const [kind, style, map] of owners) attempt(kind + ': validation', () => {
    map.set(property, 'auto');
    for (const source of ['0', '-1', '1.5', 'none', '2px', '50%', '1 2', 'auto, 2', 'var()']) {
      const before = style.cssText;
      throws(() => map.set(property, source), kind + ': invalid CSS ' + source);
      check(style.cssText === before, kind + ': atomic invalid CSS');
    }
    throws(() => map.set(property, CSS.px(2)), kind + ': wrong numeric unit');
    throws(() => map.set(property, CSS.percent(50)), kind + ': percentage is not a column count');
    throws(() => map.set(property, new CSSKeywordValue('2')), kind + ': number is not a keyword');
    throws(() => map.set(property, CSS.number(2), CSS.number(3)), kind + ': one value only');
    throws(() => map.append(property, CSS.number(2)), kind + ': cannot append');
    const opaque = CSSStyleValue.parse('grid-row-start', '2');
    throws(() => map.set(property, opaque), kind + ': opaque grid line cannot become a count');
    map.set(property, CSS.number(2)); const saved = map.get(property);
    map.set(property, new CSSUnparsedValue(['var(--columns)']));
    check(map.get(property) instanceof CSSUnparsedValue, kind + ': variable remains unparsed');
    numeric([saved], 2, kind + ': earlier snapshot survives replacement');
    map.set(property, saved); numeric(map.getAll(property), 2, kind + ': restore numeric snapshot');
  });
  attempt('inheritance and variables', () => {
    const child = document.createElement('span'); connected.append(child);
    connected.style.cssText = property + ': 5';
    keyword(child.computedStyleMap().getAll(property), 'auto', 'non-inherited initial auto');
    child.style.setProperty(property, 'inherit');
    numeric(child.computedStyleMap().getAll(property), 5, 'explicit inheritance');
    connected.style.setProperty(property, '7');
    numeric(child.computedStyleMap().getAll(property), 7, 'inherited invalidation');
    child.style.cssText = '--columns: calc(2 + 3); ' + property + ': var(--columns)';
    check(child.attributeStyleMap.get(property) instanceof CSSUnparsedValue, 'declared variable');
    numeric(child.computedStyleMap().getAll(property), 5, 'computed integer variable');
    child.style.setProperty('--columns', 'calc(-3.14)');
    numeric(child.computedStyleMap().getAll(property), 1, 'computed range restriction');
    child.style.setProperty('--columns', 'auto');
    keyword(child.computedStyleMap().getAll(property), 'auto', 'computed keyword variable');
    child.remove();
  });
  owner.remove(); connected.remove(); svg.remove();
  return {checks, cases, failures};
})())
