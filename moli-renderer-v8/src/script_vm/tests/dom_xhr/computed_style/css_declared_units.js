(() => {
  'use strict';
  const equal = (actual, expected, label) => {
    if (!Object.is(actual, expected)) throw new Error(`${label}: ${actual} !== ${expected}`);
  };
  const unit = (value, number, name, realm = globalThis) => {
    equal(Object.getPrototypeOf(value), realm.CSSUnitValue.prototype, 'unit realm');
    equal(value.unit, name, 'unit');
    equal(value.value, number, 'magnitude');
  };
  const raises = (callback, Type = TypeError) => {
    try { callback(); } catch (error) { if (error instanceof Type) return; throw error; }
    throw new Error('missing TypeError');
  };
  const other = document.getElementById('child').contentWindow;
  const detached = document.implementation.createHTMLDocument('').createElement('select');
  const elements = [
    document.createElement('div'),
    document.createElementNS('http://www.w3.org/2000/svg', 'g'),
    document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi'),
    detached,
  ];
  const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
  const foreign = other.document.createElement('div');
  const foreignSheet = new other.CSSStyleSheet(); foreignSheet.replaceSync('div {}');
  const surfaces = [
    ...elements.map(element => [element.style, element.attributeStyleMap, globalThis]),
    [sheet.cssRules[0].style, sheet.cssRules[0].styleMap, globalThis],
    [foreign.style, foreign.attributeStyleMap, other],
    [foreignSheet.cssRules[0].style, foreignSheet.cssRules[0].styleMap, other],
  ];
  for (const [style, map, realm] of surfaces) {
    for (const property of ['opacity', 'fill-opacity', 'stroke-opacity', 'flood-opacity', 'stop-opacity', 'shape-image-threshold']) {
      StylePropertyMap.prototype.set.call(map, property, other.CSS.percent(25));
      unit(map.get(property), 25, 'percent', realm);
      equal(style.getPropertyValue(property), '0.25', 'CSSOM normalizes percentages');
      style.setProperty(property, '25%');
      unit(map.get(property), 0.25, 'number', realm);
      map.set(property, '25%');
      unit(map.get(property), 0.25, 'number', realm);
      unit(CSSStyleValue.parse(property, '25%'), 0.25, 'number');
    }
    for (const number of [0, -3.14, 3.14, 200, 1.1234567891234567]) {
      const input = other.CSS.percent(number);
      map.set('opacity', input);
      const snapshot = map.get('opacity');
      unit(snapshot, number, 'percent', realm);
      unit(map.getAll('opacity')[0], number, 'percent', realm);
      unit([...map].find(([name]) => name === 'opacity')[1][0], number, 'percent', realm);
      input.value = 99; snapshot.value = 88;
      unit(map.get('opacity'), number, 'percent', realm);
      style.color = 'red';
      style.setProperty('--fallback', 'var(--other, 2px)');
      style.setProperty('background', 'var(--background)');
      style.setProperty('animation', 'fade 1s');
      unit(map.get('opacity'), number, 'percent', realm);
      style.removeProperty('background');
      equal(style.getPropertyValue('background'), '', 'unparsed shorthand removed');
      unit(map.get('opacity'), number, 'percent', realm);
      style.setProperty('-webkit-transform-origin', '20px 30px 0px');
      style.removeProperty('-webkit-transform-origin');
      equal(style.getPropertyValue('-webkit-transform-origin'), '', 'compatibility property removed');
      unit(map.get('opacity'), number, 'percent', realm);
      style.setProperty('opacity', 'invalid');
      unit(map.get('opacity'), number, 'percent', realm);
    }
    const precise = 1.1234567891234567;
    for (const [property, input] of [['opacity', CSS.number(precise)], ['margin-left', CSS.px(precise)]]) {
      map.set(property, input);
      unit(map.get(property), precise, input.unit, realm);
    }
    style.margin = '2px';
    unit(map.get('margin-left'), 2, 'px', realm);
    map.set('opacity', CSS.percent(25));
    // A valid write replaces the representation even when CSSOM text is unchanged.
    const before = style.cssText;
    style.opacity = '0.25';
    equal(style.cssText, before, 'same serialized value');
    unit(map.get('opacity'), 0.25, 'number', realm);
    const input = CSS.percent(25);
    input.toString = () => { throw new Error('author stringifier'); };
    for (const name of ['value', 'unit']) {
      Object.defineProperty(input, name, {get() { throw new Error('author getter'); }});
    }
    map.set('opacity', input);
    for (const value of [CSS.px(0), new Proxy(CSS.percent(25), {}), Object.create(CSS.percent(25))]) {
      raises(() => StylePropertyMap.prototype.set.call(map, 'opacity', value));
      unit(map.get('opacity'), 25, 'percent', realm);
    }
    raises(() => StylePropertyMap.prototype.append.call(map, 'opacity', CSS.percent(1)));
    raises(() => StylePropertyMap.prototype.set.call(map, 'opacity', CSS.percent(1), CSS.percent(2)));
    unit(map.get('opacity'), 25, 'percent', realm);
    for (const reset of [
      () => { style.opacity = ''; },
      () => style.removeProperty('all'),
      () => map.delete('opacity'),
      () => map.clear(),
      () => { style.cssText = ''; },
    ]) {
      map.set('opacity', CSS.percent(25)); reset();
      equal(map.get('opacity'), undefined, 'deletion drops typed value');
    }
    map.set('opacity', CSS.percent(25)); style.all = 'initial';
    equal(map.get('opacity').value, 'initial', 'all replaces typed value on surface ' + surfaces.findIndex(([candidate]) => candidate === style));
    style.cssText = 'opacity: 25%';
    unit(map.get('opacity'), 0.25, 'number', realm);
  }
  for (const element of elements) {
    element.style.cssText = 'cssFloat: left; -WEBKIT-USER-SELECT: text; --Token: keep;';
    equal(element.style.getPropertyValue('float'), '', 'CSS text rejects IDL accessor spelling');
    equal(element.style.getPropertyValue('user-select'), 'text', 'CSS alias normalization');
    equal(element.style.getPropertyValue('--Token'), 'keep', 'custom property case');
    equal(element.style.getPropertyValue('--token'), '', 'distinct custom property name');
  }
  const element = elements[0];
  const map = element.attributeStyleMap;
  document.body.append(element);
  for (const [number, computed] of [[-25, 0], [25, 0.25], [200, 1]]) {
    map.set('opacity', CSS.percent(number));
    unit(map.get('opacity'), number, 'percent');
    unit(element.computedStyleMap().get('opacity'), computed, 'number');
    equal(Number(getComputedStyle(element).opacity), computed, 'computed CSSOM');
  }
  const observer = new MutationObserver(() => {});
  observer.observe(element, {attributes: true, attributeOldValue: true});
  const oldStyle = element.getAttribute('style');
  map.set('opacity', CSS.percent(25));
  const records = observer.takeRecords(); observer.disconnect();
  equal(records.length, 1, 'one mutation');
  equal(records[0].oldValue, oldStyle, 'old style');
  element.setAttribute('style', 'opacity: 25%');
  unit(map.get('opacity'), 0.25, 'number');
  map.set('opacity', CSS.percent(25));
  unit(element.cloneNode().attributeStyleMap.get('opacity'), 0.25, 'number');
  element.removeAttribute('style');
  equal(map.get('opacity'), undefined, 'removed attribute');
  const heldStyle = detached.style, heldMap = detached.attributeStyleMap;
  heldMap.set('opacity', CSS.percent(25));
  document.adoptNode(detached); document.body.append(detached);
  equal(detached.style, heldStyle, 'style identity survives adoption');
  equal(detached.attributeStyleMap, heldMap, 'map identity survives adoption');
  unit(heldMap.get('opacity'), 25, 'percent');
  detached.setAttribute('style', 'opacity: 50%');
  unit(heldMap.get('opacity'), 0.5, 'number');
  equal(heldStyle.opacity, '0.5', 'held style follows the attribute');
  heldMap.set('opacity', CSS.percent(25));
  unit(detached.computedStyleMap().get('opacity'), 0.25, 'number');
  // Reactions must observe the committed representation and retain any nested write.
  const observed = [];
  let armed = false;
  customElements.define('typed-opacity-reentrant', class extends HTMLElement {
    static get observedAttributes() { return ['style']; }
    attributeChangedCallback() {
      if (!armed) return;
      const value = this.attributeStyleMap.get('opacity');
      observed.push([value.unit, value.value]);
      if (observed.length === 1) this.style.opacity = '0.75';
    }
  });
  const custom = document.createElement('typed-opacity-reentrant');
  custom.attributeStyleMap.set('opacity', CSS.percent(25));
  armed = true;
  custom.style.width = '1px';
  equal(JSON.stringify(observed), JSON.stringify([['percent', 25], ['number', 0.75]]), 'reentrant observation');
  unit(custom.attributeStyleMap.get('opacity'), 0.75, 'number');
  equal(custom.style.opacity, '0.75', 'reentrant CSSOM value');
  element.remove(); detached.remove();
  return true;
})()
