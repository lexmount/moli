(() => {
  const equal = (actual, expected, label) => {
    if (!Object.is(actual, expected)) throw new Error(`${label}: ${actual} !== ${expected}`);
  };
  const raises = (callback, Type = TypeError) => {
    try { callback(); } catch (error) { if (error instanceof Type) return; throw error; }
    throw new Error('expected ' + Type.name);
  };
  const unit = (value, number, Unit = CSSUnitValue) => {
    equal(Object.getPrototypeOf(value), Unit.prototype, 'native unit prototype');
    equal(value.unit, 'percent', 'percentage unit');
    equal(value.value, number, 'percentage magnitude');
  };
  const other = document.getElementById('child').contentWindow;
  const elements = [
    document.createElement('div'),
    document.createElementNS('http://www.w3.org/2000/svg', 'g'),
    document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'),
    document.implementation.createHTMLDocument('').createElement('select'),
  ];
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { width: 1px; }');
  const foreign = other.document.createElement('div');
  const foreignSheet = new other.CSSStyleSheet();
  foreignSheet.replaceSync('div { width: 1px; }');
  const surfaces = [
    ...elements.map(element => [element.style, element.attributeStyleMap, CSSUnitValue]),
    [sheet.cssRules[0].style, sheet.cssRules[0].styleMap, CSSUnitValue],
    [foreign.style, foreign.attributeStyleMap, other.CSSUnitValue],
    [foreignSheet.cssRules[0].style, foreignSheet.cssRules[0].styleMap, other.CSSUnitValue],
  ];
  for (const [style, map, Unit] of surfaces) {
    for (const property of ['width', 'height', 'min-width', 'max-width', 'padding-left', 'font-size', 'line-height']) {
      for (const number of [0, 3.125, 100]) {
        const input = other.CSS.percent(number);
        StylePropertyMap.prototype.set.call(map, property, input);
        equal(style.getPropertyValue(property), number + '%', 'CSSOM percentage');
        const snapshot = map.get(property);
        unit(snapshot, number, Unit);
        equal(map.getAll(property).length, 1, 'single percentage');
        unit(map.getAll(property)[0], number, Unit);
        input.value = 999;
        snapshot.value = 888;
        unit(map.get(property), number, Unit);
      }
    }
    for (const property of ['left', 'margin-left', 'text-indent']) {
      map.set(property, CSS.percent(-12.5));
      unit(map.get(property), -12.5, Unit);
      equal(style.getPropertyValue(property), '-12.5%', 'unrestricted negative percentage');
    }
    style.setProperty('width', '2px', 'important');
    const input = CSS.percent(25);
    input.toString = () => { throw new Error('author toString'); };
    for (const name of ['value', 'unit']) {
      Object.defineProperty(input, name, {get() { throw new Error('author ' + name); }});
    }
    map.set('WIDTH', input);
    unit(map.get('width'), 25, Unit);
    equal(style.getPropertyPriority('width'), '', 'typed write removes priority');
    unit([...map].find(([name]) => name === 'width')[1][0], 25, Unit);
    const before = style.cssText;
    for (const [property, input] of [
      ['width', CSS.number(0)], ['width', CSS.s(0)],
      ['border-left-width', CSS.percent(0)], ['z-index', CSS.percent(1)],
      ['transition-duration', CSS.percent(1)], ['margin', CSS.percent(1)],
    ]) {
      raises(() => StylePropertyMap.prototype.set.call(map, property, input));
      equal(style.cssText, before, 'invalid typed write remains atomic');
    }
    raises(() => StylePropertyMap.prototype.append.call(map, 'width', CSS.percent(1)));
    raises(() => StylePropertyMap.prototype.set.call(map, 'width', CSS.percent(1), CSS.percent(2)));
    equal(style.cssText, before, 'non-list writes remain atomic');
    map.set('transition-duration', '1s');
    const durationBefore = style.cssText;
    raises(() => StylePropertyMap.prototype.append.call(map, 'transition-duration', CSS.s(2), CSS.percent(1)));
    equal(style.cssText, durationBefore, 'invalid appended percentage remains atomic');
  }
  const element = document.createElement('div');
  document.body.append(element);
  const map = element.attributeStyleMap;
  map.set('width', CSS.percent(25));
  unit(element.computedStyleMap().get('width'), 25);
  const observer = new MutationObserver(() => {});
  observer.observe(element, {attributes: true, attributeOldValue: true});
  const oldStyle = element.getAttribute('style');
  map.set('width', CSS.percent(50));
  const records = observer.takeRecords();
  observer.disconnect();
  equal(records.length, 1, 'one mutation for a typed percentage');
  equal(records[0].oldValue, oldStyle, 'mutation old value');
  unit(element.computedStyleMap().get('width'), 50);
  raises(() => other.StylePropertyMap.prototype.set.call(map, 'width', other.CSS.number(0)), other.TypeError);
  const parsed = CSSStyleValue.parse('width', '75%');
  map.set('width', parsed);
  unit(map.get('width'), 75);
  map.set('width', CSSNumericValue.parse('12.5%'));
  unit(map.get('width'), 12.5);
  const marker = new Error('property conversion');
  try { map.set({toString() {throw marker;}}, CSS.percent(1)); throw new Error('missing exception'); }
  catch (error) { equal(error, marker, 'original conversion exception'); }
  unit(map.get('width'), 12.5);
  let conversions = 0;
  raises(() => StylePropertyMap.prototype.set.call({}, {toString() {conversions++; return 'width';}}, CSS.percent(1)));
  equal(conversions, 0, 'receiver check before conversion');
  element.remove();
  return true;
})()
