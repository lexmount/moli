JSON.stringify((() => {
  const failures = [];
  const cases = [];
  let checks = 0;
  const parent = document.createElement('div');
  parent.style.cssText = 'font-size: 20px; line-height: 24px';
  const element = document.createElement('span');
  element.id = 'target';
  parent.append(element);
  document.body.append(parent);
  const property = 'vertical-align';
  const specified = element.attributeStyleMap;
  const computed = element.computedStyleMap();
  const cssom = getComputedStyle(element);
  const check = (condition, label) => {
    checks++;
    if (!condition) failures.push(label);
  };
  const unit = (value, number, name) => value instanceof CSSUnitValue &&
    value.unit === name && Math.abs(value.value - number) < 0.0001;
  const expectedType = {
    keyword: (value, text) => value instanceof CSSKeywordValue && value.value === text,
    px: (value, text) => unit(value, parseFloat(text), 'px'),
    percent: (value, text) => unit(value, parseFloat(text), 'percent'),
    math: value => value instanceof CSSMathSum,
    compound: value => Object.prototype.toString.call(value) === '[object CSSStyleValue]'
  };
  const read = (label, text, type) => {
    const value = computed.get(property);
    check(cssom.getPropertyValue(property) === text, label + ' CSSOM: ' + cssom.verticalAlign);
    check(expectedType[type](value, text), label + ' computed type: ' + String(value));
    const all = computed.getAll(property);
    check(all.length === 1 && expectedType[type](all[0], text), label + ' getAll');
    check(computed.has(property), label + ' has');
    cases.push(label);
  };
  read('initial', 'baseline', 'keyword');
  check(cssom.getPropertyValue('baseline-shift') === '0px', 'initial longhand CSSOM');
  check(unit(computed.get('baseline-shift'), 0, 'px'), 'initial longhand Typed OM');
  for (const [source, text, type] of [
    ['baseline', 'baseline', 'keyword'],
    ['0px', '0px', 'px'],
    ['0em', '0px', 'px'],
    ['calc(0px + 0em)', '0px', 'px'],
    ['-3em', '-60px', 'px'],
    ['0%', '0%', 'percent'],
    ['25%', '25%', 'percent'],
    ['calc(0% + 0%)', '0%', 'percent'],
    ['calc(10% + 5px)', 'calc(10% + 5px)', 'math'],
    ['super', 'super', 'keyword'],
    ['middle', 'middle', 'keyword'],
    ['first', 'first', 'keyword'],
    ['first 0px', 'first 0px', 'compound'],
    ['last alphabetic -2px', 'last alphabetic -2px', 'compound']
  ]) {
    for (const writer of ['CSSOM', 'Typed OM']) {
      element.style.cssText = '';
      if (writer === 'CSSOM') element.style.setProperty(property, source);
      else specified.set(property, source);
      read(writer + ' ' + source, text, type);
      if (source === '0px') {
        check(unit(specified.get(property), 0, 'px'), writer + ' specified zero');
        check(element.style.getPropertyValue(property) === '0px', writer + ' serialized zero');
      }
    }
  }

  specified.set(property, CSS.px(0));
  read('typed zero', '0px', 'px');
  specified.set(property, new CSSKeywordValue('baseline'));
  read('typed baseline after zero', 'baseline', 'keyword');
  element.style.cssText = 'baseline-source: first; alignment-baseline: text-bottom; baseline-shift: 2em';
  read('independent longhands', 'first text-bottom 40px', 'compound');
  const snapshot = computed.get(property);
  element.style.cssText = '';
  read('removed longhands', 'baseline', 'keyword');
  check(String(snapshot) === 'first text-bottom 40px', 'computed snapshot');
  specified.set(property, snapshot);
  read('write compound snapshot', 'first text-bottom 40px', 'compound');

  element.style.cssText = 'vertical-align: inherit';
  parent.style.verticalAlign = '0px';
  read('inherited explicit zero', '0px', 'px');
  parent.style.verticalAlign = 'baseline';
  read('inherited baseline', 'baseline', 'keyword');
  element.style.cssText = 'vertical-align: var(--shift); --shift: 0px';
  read('substituted explicit zero', '0px', 'px');
  element.style.setProperty('--shift', 'baseline');
  read('substituted baseline', 'baseline', 'keyword');

  element.style.cssText = '';
  const owner = document.createElement('style');
  owner.textContent = '#target { vertical-align: 0px }';
  document.head.append(owner);
  read('stylesheet zero', '0px', 'px');
  owner.sheet.cssRules[0].style.setProperty(property, 'baseline');
  read('stylesheet baseline', 'baseline', 'keyword');
  owner.remove();
  parent.remove();
  return {checks, cases, failures};
})())
