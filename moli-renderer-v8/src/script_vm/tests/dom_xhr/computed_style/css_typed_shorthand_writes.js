(() => {
  const result = {checks: 0, failures: [], cases: []};
  function check(ok, label, detail) { result.checks++; if (!ok) result.failures.push({label, detail}); }
  const shorthands = [
    ['background', 'red'], ['mask', 'none'], ['border', '1px solid red'],
    ['border-width', '1px'], ['border-radius', '1px'], ['margin', '1px'],
    ['padding', '1px'], ['inset', '1px'], ['gap', '1px'],
    ['grid-area', '1'], ['grid-row', '1'], ['grid-column', '1'],
    ['columns', '1px'], ['outline', '1px solid red'], ['text-decoration', 'underline'],
    ['animation', 'none'], ['transition', 'none'], ['flex', '1'], ['font', '12px serif']
  ];
  const inputs = [
    () => new CSSMathSum(CSS.px(0), CSS.em(0)),
    () => new CSSMathSum(CSS.percent(0), CSS.percent(0)),
    () => new CSSMathSum(CSS.number(2), CSS.number(3)),
    () => new CSSMathSum(CSS.s(0), CSS.ms(0)),
    () => new CSSMathMin(CSS.px(0), CSS.em(0)),
    () => new CSSMathProduct(CSS.px(0), CSS.number(2)),
    () => CSS.px(0), () => CSS.percent(0)
  ];
  const sheet = new CSSStyleSheet(); sheet.replaceSync('.fixture {}');
  for (const owner of ['inline', 'rule']) {
    for (const [property, value] of shorthands) {
      const element = document.createElement('div');
      const style = owner === 'inline' ? element.style : sheet.cssRules[0].style;
      const map = owner === 'inline' ? element.attributeStyleMap : sheet.cssRules[0].styleMap;
      style.cssText = 'color:red;--sentinel:keep';
      map.set(property, value);
      check(style.getPropertyValue(property) !== '', owner + ':' + property + ':string accepted', style.cssText);
      const parsed = CSSStyleValue.parse(property, value);
      map.set(property, parsed);
      check(style.getPropertyValue(property) !== '', owner + ':' + property + ':opaque value accepted', style.cssText);
      for (const make of inputs) {
        const input = make(); const before = style.cssText; let error = null;
        try { map.set(property, input); } catch (e) { error = e.name; }
        const label = owner + ':' + property + ':' + input.constructor.name + ':' + String(input);
        check(error === 'TypeError', label + ':reject numeric object', {error, after: style.cssText});
        check(style.cssText === before, label + ':atomic rejection', {before, after: style.cssText});
        result.cases.push({owner, property, input: String(input), type: input.constructor.name, error});
        style.cssText = before;
      }
    }
  }
  for (const [property, make] of [
    ['width', () => new CSSMathSum(CSS.px(2), CSS.em(3))],
    ['font-size', () => new CSSMathSum(CSS.px(2), CSS.em(3))],
    ['opacity', () => new CSSMathSum(CSS.number(.2), CSS.number(.3))],
    ['line-height', () => new CSSMathSum(CSS.number(2), CSS.number(3))],
    ['transition-delay', () => new CSSMathSum(CSS.s(2), CSS.ms(3))]
  ]) {
    const el = document.createElement('div'); let error = null;
    try { el.attributeStyleMap.set(property, make()); } catch(e) { error = e.name; }
    check(error === null && el.style.getPropertyValue(property) !== '', property + ':valid numeric longhand', {error, cssText: el.style.cssText});
  }
  for (const owner of ['inline', 'rule']) {
    const el = document.createElement('div');
    const map = owner === 'inline' ? el.attributeStyleMap : sheet.cssRules[0].styleMap;
    for (const input of [CSS.px(0), CSS.percent(0), new CSSMathSum(CSS.px(0), CSS.em(0)),
      new CSSMathSum(CSS.percent(0), CSS.percent(0)), new CSSKeywordValue('baseline'), new CSSKeywordValue('inherit')]) {
      let error = null, output = null;
      try { map.set('vertical-align', input); output = map.get('vertical-align'); } catch(e) { error = e.name; }
      check(error === null, owner + ':vertical-align:valid scalar accepted', {input: String(input), error});
      check(output && output.constructor.name === input.constructor.name,
        owner + ':vertical-align:scalar type preserved', {input: input.constructor.name, output: output && output.constructor.name});
      const parsed = CSSStyleValue.parse('vertical-align', String(input));
      const equivalentScalar = input instanceof CSSMathSum && parsed instanceof CSSUnitValue &&
        input.to(parsed.unit).value === parsed.value;
      check(parsed.constructor.name === input.constructor.name || equivalentScalar,
        owner + ':vertical-align:parsed scalar type preserved', {input: String(input), parsed: String(parsed), type: parsed.constructor.name});
    }
  }
  return JSON.stringify(result);
})()
