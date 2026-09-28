(() => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  const throws = (callback, name) => {
    let error;
    try {callback();} catch (e) {error=e;}
    check(error && error.name === name, name + ' required');
  };
  const ns = 'http://www.w3.org/2000/svg';
  for (const doc of [document, document.implementation.createHTMLDocument(),
    document.implementation.createDocument(ns, 'svg')]) {
    for (const [tag, name, initial] of [['rect','x','0'],['filter','width','120%'],
      ['feBlend','width','100%'],['svg','width','100%'],['circle','cx','0']]) {
      const element = doc.createElementNS(ns, tag);
      const animated = element[name], base = animated.baseVal, anim = animated.animVal;
      check(base.valueAsString === initial && anim.valueAsString === initial, tag + ' defaults');
      element.setAttribute(name, '12px');
      check(base.value === 12 && anim.value === 12 && base.unitType === 5, tag + ' retained values update');
      base.valueInSpecifiedUnits = {valueOf() {element.setAttribute(name, '3cm');return 8;}};
      check(element.getAttribute(name) === '8cm' && base.unitType === 6 && anim.unitType === 6, tag + ' read after argument conversion');
      base.newValueSpecifiedUnits(5, 6);
      check(element.getAttribute(name) === '6px' && anim.value === 6, tag + ' new units');
      base.convertToSpecifiedUnits(1);
      check(element.getAttribute(name) === '6' && anim.valueAsString === '6', tag + ' converted units');
      let converted = 0;
      throws(() => {anim.value = {valueOf() {converted++;return 4;}};}, 'NoModificationAllowedError');
      check(converted === 1, 'WebIDL conversion precedes read-only check');
      throws(() => {anim.valueInSpecifiedUnits = 4;}, 'NoModificationAllowedError');
      throws(() => {anim.valueAsString = '4';}, 'NoModificationAllowedError');
      throws(() => anim.newValueSpecifiedUnits(1, 4), 'NoModificationAllowedError');
      throws(() => anim.convertToSpecifiedUnits(1), 'NoModificationAllowedError');
      check(element.getAttribute(name) === '6', 'read-only operations do not mutate');
      element.removeAttribute(name);
      check(base.valueAsString === initial && anim.valueAsString === initial, tag + ' defaults after removal');
    }
    const element = doc.createElementNS(ns, 'g');
    const value = element.className;
    element.setAttribute('class', 'before\uD800after');
    check(value.baseVal === 'before\uD800after' && value.animVal === 'before\uD800after', 'read DOMString code units');
    let conversions = 0;
    value.baseVal = {toString() {conversions++;return '\uDC00';}};
    check(conversions === 1 && value.baseVal === '\uDC00' && value.animVal === '\uDC00' && element.getAttribute('class') === '\uDC00', 'write DOMString code units');
    const error = {};
    let caught;
    try {value.baseVal = {toString() {throw error;}};} catch (e) {caught=e;}
    check(caught === error && value.baseVal === '\uDC00', 'conversion exception preserves attribute');
    throws(() => {value.baseVal = Symbol();}, 'TypeError');
  }
  return true;
})()
