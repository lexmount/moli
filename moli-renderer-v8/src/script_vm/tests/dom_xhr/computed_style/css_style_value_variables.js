(() => {
  const equal = (actual, expected, label) => {
    if (!Object.is(actual, expected)) throw new Error(`${label}: ${actual} !== ${expected}`);
  };
  // Current Typed OM requirements differ from older Chromium's shorthand and
  // empty fallback handling. Keep these normative cases explicit.
  for (const property of ['width', 'margin', 'transition-duration', '--custom']) {
    const source = 'calc(42px + var(--A, env(foo, var(--B,))))';
    const value = CSSStyleValue.parse(property, source);
    equal(value instanceof CSSUnparsedValue, true, property + ' unparsed type');
    equal(value.length, 3, 'segments around variable');
    equal(value[0], 'calc(42px + ', 'leading tokens');
    equal(value[2], ')', 'trailing tokens');
    const outer = value[1];
    equal(outer.variable, '--A', 'outer variable');
    equal(outer.fallback[0], ' env(foo, ', 'non-var function retained');
    const inner = outer.fallback[1];
    equal(inner.variable, '--B', 'inner variable');
    equal(inner.fallback instanceof CSSUnparsedValue, true, 'explicit empty fallback');
    equal(inner.fallback.length, 0, 'empty fallback length');
    equal(String(value), source, 'nested roundtrip');
    const list = CSSStyleValue.parseAll(property, source);
    equal(list.length, 1, 'var list stays unparsed');
    equal(list[0] instanceof CSSUnparsedValue, true, 'parseAll native type');
  }
  for (const source of ['env(foo)', 'env(foo, bar)', 'attr(data-x)', 'initial', 'a/**/b']) {
    const value = CSSStyleValue.parse('--x', source);
    equal(value instanceof CSSUnparsedValue, true, 'custom tokens');
    equal(value.length, 1, 'raw functions are not variable references');
    equal(String(value), source, 'raw token boundaries');
  }
  equal(CSSStyleValue.parse('--\uD800', '\uDC00')[0], '\uFFFD', 'USVString conversion');
  equal(String(CSSStyleValue.parse('width', 'v\\61 r(--\\41 )')), 'var(--A)', 'escaped identifiers');
  let source = 'leaf';
  for (let i = 0; i < 60; ++i) source = 'var(--x,' + source + ')';
  equal(String(CSSStyleValue.parse('--deep', source)), source, 'deep fallback projection');
  return true;
})()
