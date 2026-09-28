(() => {
  const equal = (a, b, label) => { if (!Object.is(a, b)) throw new Error(`${label}: ${a} !== ${b}`); };
  const raises = (fn, Type = TypeError) => { try { fn(); } catch (e) { if (e instanceof Type) return; throw e; } throw new Error('expected ' + Type.name); };
  const other = document.getElementById('child').contentWindow;
  const elements = [document.createElement('div'), document.createElementNS('http://www.w3.org/2000/svg', 'g'), document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'), document.implementation.createHTMLDocument('').createElement('select')];
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { width: 2px; }');
  const rule = sheet.cssRules[0];
  equal(rule.styleMap, rule.styleMap, 'rule SameObject');
  for (const element of elements) equal(element.attributeStyleMap, element.attributeStyleMap, 'inline SameObject');
  equal(Object.getPrototypeOf(StylePropertyMap.prototype), StylePropertyMapReadOnly.prototype, 'interface inheritance');
  raises(() => new StylePropertyMap());
  for (const name of ['set', 'append', 'delete', 'clear']) {
    const descriptor = Object.getOwnPropertyDescriptor(StylePropertyMap.prototype, name);
    equal(descriptor.enumerable && descriptor.writable && descriptor.configurable, true, name + ' descriptor');
    equal(descriptor.value.length, name === 'clear' ? 0 : 1, name + ' arity');
  }
  for (const [style, map] of [...elements.map(el => [el.style, el.attributeStyleMap]), [rule.style, rule.styleMap]]) {
    equal(map instanceof StylePropertyMap, true, 'mutable brand');
    equal(map instanceof StylePropertyMapReadOnly, true, 'base brand');
    style.cssText = '--first: auto; width: 10px; transition-duration: 1s, 2s;';
    equal([...map.keys()].join('|'), '--first|width|transition-duration', 'declaration order');
    equal(map.size, 3, 'declaration count');
    equal(map.get('--first') instanceof CSSUnparsedValue, true, 'custom type');
    equal(map.get('wIdTh').value, 10, 'unit value');
    equal(map.getAll('transition-duration').map(String).join('|'), '1s|2s', 'list reification');
    equal(map.get('height'), undefined, 'absent property');
    equal(map.has('height'), false, 'absent has');
    map.set('width', other.CSS.px(5));
    equal(style.width, '5px', 'foreign native value');
    const typed = CSS.px(7);
    typed.toString = () => { throw new Error('author toString'); };
    Object.defineProperty(typed, 'value', { get() { throw new Error('author value'); } });
    Object.defineProperty(typed, 'unit', { get() { throw new Error('author unit'); } });
    Object.defineProperty(style, 'setProperty', { configurable: true, value() { throw new Error('author CSSOM method'); } });
    map.set('width', typed);
    equal(style.width, '7px', 'native slots bypass author JS');
    delete style.setProperty;
    map.set('transition-duration', CSS.s(3), '4s, 5s');
    map.append('transition-duration', CSS.s(6));
    equal(map.getAll('transition-duration').map(String).join('|'), '3s|4s|5s|6s', 'set and append');
    const before = style.cssText;
    for (const [property, values] of [
      ['width', [CSS.deg(0)]], ['width', [CSS.number(0)]], ['width', ['10s']],
      ['width', [CSS.px(1), CSS.px(2)]], ['--x', [CSS.px(1)]],
      ['width', [new CSSKeywordValue('1px')]], ['src', ['url(a)']],
      ['padding', [CSSStyleValue.parse('margin', '1px')]],
      ['transition-duration', ['1s', 'var(--x)']],
    ]) {
      raises(() => map.set(property, ...values));
      equal(style.cssText, before, 'failed set is atomic: ' + property);
    }
    raises(() => map.append('transition-duration', '1s', '2px'));
    raises(() => map.append('width', CSS.px(2)));
    equal(style.cssText, before, 'failed append is atomic');
    map.set('--x', new CSSUnparsedValue(['var(--value, 2px)']));
    equal(map.get('--x') instanceof CSSUnparsedValue, true, 'unparsed custom');
    map.set('width', new CSSUnparsedValue(['var(--value, 2px)']));
    equal(map.get('width') instanceof CSSUnparsedValue, true, 'unparsed standard');
    style.setProperty('width', '9px', 'important');
    map.set('width', '11px');
    equal(style.getPropertyPriority('width'), '', 'typed write removes important');
    map.set('margin', '1px 2px');
    const shorthand = map.get('margin');
    equal(shorthand.constructor, CSSStyleValue, 'opaque shorthand');
    map.delete('margin');
    equal(map.has('margin-top'), false, 'delete expands shorthand');
    map.set('margin', shorthand);
    equal(style.margin, '1px 2px', 'opaque round trip');
    for (const keyword of ['initial', 'inherit', 'unset', 'revert', 'revert-layer']) {
      map.set('margin', new CSSKeywordValue(keyword));
      equal(String(map.get('margin')), keyword, 'CSS-wide shorthand');
    }
    style.width = '12px';
    equal(map.get('width').value, 12, 'CSSOM mutation is live');
    map.clear();
    equal(style.cssText, '', 'clear declaration');
    equal(map.size, 0, 'clear size');
    equal([...map].length, 0, 'empty iterator');
    map.append('width');
    equal(map.size, 0, 'empty append');
    raises(() => map.set('width'));
  }
  const el = elements[0], map = el.attributeStyleMap;
  let conversions = 0;
  const input = { toString() { conversions++; return 'width'; } };
  const revoked = Proxy.revocable(map, {}); revoked.revoke();
  for (const fake of [{}, Object.create(map), new Proxy(map, {}), revoked.proxy]) {
    raises(() => StylePropertyMap.prototype.set.call(fake, input, input));
    raises(() => StylePropertyMapReadOnly.prototype.get.call(fake, input));
  }
  equal(conversions, 0, 'brand before conversions');
  const order = [];
  raises(() => map.set({toString() {order.push('property'); return 'invalid-property';}}, {toString() {order.push('a'); return '1px';}}, {toString() {order.push('b'); return '2px';}}));
  equal(order.join('|'), 'property|a|b', 'convert all union values before validation');
  const marker = {};
  try { map.set('width', {toString() {throw marker;}}); throw new Error('missing conversion exception'); }
  catch (e) { equal(e, marker, 'conversion exception identity'); }
  raises(() => other.StylePropertyMap.prototype.set.call({}, 'width', '1px'), other.TypeError);
  raises(() => other.StylePropertyMap.prototype.set.call(map, 'width', '1s'), other.TypeError);
  const foreign = other.document.createElement('div');
  foreign.attributeStyleMap.set('width', '2px');
  equal(Object.getPrototypeOf(foreign.attributeStyleMap), other.StylePropertyMap.prototype, 'foreign owner map');
  equal(Object.getPrototypeOf(foreign.attributeStyleMap.get('width')), other.CSSUnitValue.prototype, 'foreign returned value');
  const freshLocal = document.createElement('div');
  const foreignGetter = Object.getOwnPropertyDescriptor(other.HTMLElement.prototype, 'attributeStyleMap').get;
  equal(Object.getPrototypeOf(foreignGetter.call(freshLocal)), StylePropertyMap.prototype, 'borrowed getter owner realm');
  foreign.attributeStyleMap.set('width', '2px');
  equal(Object.getPrototypeOf(StylePropertyMapReadOnly.prototype.get.call(foreign.attributeStyleMap, 'width')), other.CSSUnitValue.prototype, 'borrowed method owner value realm');
  const freshSheet = new CSSStyleSheet(); freshSheet.replaceSync('p { width: 1px; }');
  const foreignRuleGetter = Object.getOwnPropertyDescriptor(other.CSSStyleRule.prototype, 'styleMap').get;
  equal(Object.getPrototypeOf(foreignRuleGetter.call(freshSheet.cssRules[0])), StylePropertyMap.prototype, 'borrowed rule getter owner realm');
  const NativeMap = StylePropertyMap, NativeUnit = CSSUnitValue;
  try {
    globalThis.StylePropertyMap = globalThis.CSSUnitValue = function() {throw marker;};
    const fresh = document.createElement('div').attributeStyleMap;
    fresh.set('width', '3px');
    equal(Object.getPrototypeOf(fresh), NativeMap.prototype, 'native map constructor');
    equal(Object.getPrototypeOf(fresh.get('width')), NativeUnit.prototype, 'native reification');
  } finally { globalThis.StylePropertyMap = NativeMap; globalThis.CSSUnitValue = NativeUnit; }
  el.style.cssText = 'width: 1px; height: 2px;';
  const iterator = map.keys();
  map.set('opacity', '.5');
  equal([...iterator].join('|'), 'width|height', 'iterator snapshot');
  const receiver = {};
  let visited = 0;
  map.forEach(function(values, key, source) {
    equal(this, receiver, 'forEach thisArg'); equal(source, map, 'forEach map');
    equal(Array.isArray(values), true, 'forEach sequence'); equal(typeof key, 'string', 'forEach key'); visited++;
  }, receiver);
  equal(visited, 3, 'forEach count');
  const records = new MutationObserver(() => {}); records.observe(el, {attributes: true, attributeOldValue: true});
  const old = el.getAttribute('style');
  map.set('width', '4px');
  const changes = records.takeRecords(); records.disconnect();
  equal(changes.length, 1, 'one attribute mutation'); equal(changes[0].oldValue, old, 'attribute old value');
  equal(String(new CSSKeywordValue('1px')), '\\31 px', 'identifier serialization');
  return true;
})()
