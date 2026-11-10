JSON.stringify((() => {
  const failures = [];
  const cases = [];
  let checks = 0;
  const check = (condition, label) => {
    checks++;
    if (!condition) failures.push(label);
  };
  const attempt = (label, callback) => {
    try { callback(); } catch (error) { failures.push(label + ': ' + error); }
  };
  const keyword = (values, expected, label) => check(values.length === 1 &&
    values[0] instanceof CSSKeywordValue && values[0].value === expected &&
    String(values[0]) === CSS.escape(expected), label + ': keyword ' + values.map(String));
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
  const owner = document.createElement('style'); owner.textContent = '.scoped-names {}'; document.head.append(owner);
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.scoped-names {}');
  const owners = [
    ['connected', connected.style, connected.attributeStyleMap],
    ['detached', detached.style, detached.attributeStyleMap],
    ['svg', svg.style, svg.attributeStyleMap],
    ['style-rule', owner.sheet.cssRules[0].style, owner.sheet.cssRules[0].styleMap],
    ['constructed-rule', sheet.cssRules[0].style, sheet.cssRules[0].styleMap]
  ];
  for (const property of ['anchor-scope', 'timeline-scope']) {
    for (const [source, expected] of [
      ['none', 'none'], ['NONE', 'none'], ['n\\6f ne', 'none'],
      ['all', 'all'], ['ALL', 'all'], ['\\61ll', 'all'],
      ['--a', '--a'], ['--A', '--A'], ['--é', '--é'],
      ['--foo\\2e bar', '--foo.bar'], ['--foo\\20 bar', '--foo bar'],
      ['--foo\\5c bar', '--foo\\bar']
    ]) {
      const prefix = property + ': ' + source;
      attempt(prefix + ': parsing', () => {
        keyword([CSSStyleValue.parse(property, source)], expected, prefix + ': parse');
        keyword(CSSStyleValue.parseAll(property, source), expected, prefix + ': parseAll');
      });
      for (const [kind, style, map] of owners) {
        for (const write of ['string', 'keyword', 'parsed', 'cssom', 'cssText']) {
          const label = [property, source, kind, write].join(': ');
          attempt(label, () => {
            style.cssText = 'color: red';
            if (write === 'string') map.set(property, source);
            if (write === 'keyword') map.set(property, new CSSKeywordValue(expected));
            if (write === 'parsed') map.set(property, CSSStyleValue.parse(property, source));
            if (write === 'cssom') style.setProperty(property, source);
            if (write === 'cssText') style.cssText = property + ': ' + source;
            keyword(map.getAll(property), expected, label + ': getAll');
            keyword([map.get(property)], expected, label + ': get');
            keyword(Array.from(map).find(([name]) => name === property)[1], expected, label + ': iteration');
            style.setProperty('opacity', '0.5');
            keyword(map.getAll(property), expected, label + ': unrelated write');
            check(style.getPropertyValue(property) === CSS.escape(expected), label + ': CSSOM escapes exactly once');
            if (kind === 'style-rule' || kind === 'constructed-rule') {
              const rule = kind === 'style-rule' ? owner.sheet.cssRules[0] : sheet.cssRules[0];
              keyword(rule.styleMap.getAll(property), expected, label + ': reacquired rule');
            }
            cases.push(label);
          });
        }
      }
      attempt(prefix + ': computed', () => {
        connected.style.cssText = property + ': ' + source;
        keyword(connected.computedStyleMap().getAll(property), expected, prefix + ': computed');
      });
    }
    for (const source of ['--a, --b', '--a, --a', '--A, --a', '--foo\\2e bar, --é']) {
      const prefix = property + ': ' + source;
      attempt(prefix + ': parsing', () => opaque(CSSStyleValue.parseAll(property, source), prefix));
      for (const [kind, style, map] of owners) attempt(prefix + ': ' + kind, () => {
        const parsed = CSSStyleValue.parse(property, source);
        style.cssText = 'color: red'; map.set(property, parsed);
        opaque(map.getAll(property), prefix + ': ' + kind);
        const text = style.getPropertyValue(property);
        check(text.includes(','), prefix + ': comma retained');
        style.setProperty('opacity', '0.25');
        opaque(map.getAll(property), prefix + ': after mutation');
        check(String(map.get(property)) === text, prefix + ': entire list retained');
        map.set(property, new CSSKeywordValue(source));
        keyword(map.getAll(property), source, prefix + ': punctuation in one identifier is escaped');
        map.set(property, parsed);
        check(style.getPropertyValue(property) === text, prefix + ': opaque list restored after escaped identifier');
        check(style.getPropertyValue('color') === 'red' && style.getPropertyValue('opacity') === '0.25', prefix + ': unrelated properties retained');
        const before = style.cssText;
        throws(() => map.set(property, new CSSKeywordValue('--a'), new CSSKeywordValue('--b')), prefix + ': list cannot be typed arguments');
        throws(() => map.append(property, new CSSKeywordValue('--c')), prefix + ': list cannot be appended');
        check(style.cssText === before, prefix + ': atomic rejection');
        const other = property === 'anchor-scope' ? 'timeline-scope' : 'anchor-scope';
        throws(() => map.set(other, parsed), prefix + ': opaque association');
        map.set(property, new CSSKeywordValue('--A'));
        keyword(map.getAll(property), '--A', prefix + ': restore keyword');
      });
      attempt(prefix + ': computed', () => {
        connected.style.cssText = property + ': ' + source;
        opaque(connected.computedStyleMap().getAll(property), prefix + ': computed opaque list');
      });
    }
    for (const [kind, style, map] of owners) attempt(property + ': ' + kind + ': validation', () => {
      map.set(property, '--A');
      const saved = map.get(property);
      for (const invalid of ['foo', 'all, --a', 'none, --a', '--a --b', '--a,', 'var()']) {
        const before = style.cssText;
        throws(() => map.set(property, invalid), property + ': invalid ' + invalid);
        check(style.cssText === before, property + ': invalid write is atomic');
      }
      map.set(property, new CSSUnparsedValue(['var(--scope)']));
      check(map.get(property) instanceof CSSUnparsedValue, property + ': variable stays unparsed');
      map.set(property, new CSSKeywordValue('inherit'));
      keyword(map.getAll(property), 'inherit', property + ': CSS-wide keyword');
      map.set(property, saved);
      keyword(map.getAll(property), '--A', property + ': immutable saved value');
    });
    attempt(property + ': cascade and variables', () => {
      const child = document.createElement('span'); connected.append(child);
      connected.style.cssText = property + ': --parent';
      keyword(child.computedStyleMap().getAll(property), 'none', property + ': non-inherited default');
      child.style.cssText = property + ': inherit';
      keyword(child.computedStyleMap().getAll(property), '--parent', property + ': explicit inheritance');
      child.style.cssText = '--scope: --A; ' + property + ': var(--scope)';
      check(child.attributeStyleMap.get(property) instanceof CSSUnparsedValue, property + ': declared variable');
      keyword(child.computedStyleMap().getAll(property), '--A', property + ': computed variable');
      child.style.setProperty('--scope', '--a, --b');
      opaque(child.computedStyleMap().getAll(property), property + ': computed list variable');
      child.style.setProperty('--scope', '--a');
      keyword(child.computedStyleMap().getAll(property), '--a', property + ': variable invalidation');
      child.remove();
    });
  }
  owner.remove(); connected.remove(); svg.remove();
  return {checks, cases, failures};
})())
