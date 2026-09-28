(() => {
  const equal = (a, b, label) => {
    if (!Object.is(a, b)) throw new Error(`${label}: ${a} !== ${b}`);
  };
  let serial = 0;
  for (const write of [
    element => { element.style.width = '25px'; },
    element => element.style.setProperty('width', '25px'),
    element => { element.style.cssText = 'width: 25px'; },
  ]) {
    const observed = [];
    const name = `style-reactions-${serial++}`;
    customElements.define(name, class extends HTMLElement {
      static get observedAttributes() { return ['style']; }
      attributeChangedCallback() {
        observed.push([this.style.width, this.attributeStyleMap.get('width').value]);
        if (observed.length === 1) this.style.width = '75px';
      }
    });
    const element = document.createElement(name);
    write(element);
    equal(JSON.stringify(observed), JSON.stringify([['25px', 25], ['75px', 75]]), 'callback values');
    equal(element.style.width, '75px', 'reentrant CSSOM value');
    equal(element.attributeStyleMap.get('width').value, 75, 'reentrant typed value');
    equal(element.getAttribute('style'), 'width: 75px;', 'reflected attribute');
  }
  return true;
})()
