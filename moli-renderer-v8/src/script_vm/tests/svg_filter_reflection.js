(async () => {
  const checks = [];
  const run = (name, callback) => {
    try {checks.push({name, passed: !!callback(), detail: null});}
    catch (error) {checks.push({name, passed: false, detail: String(error)});}
  };
  const caught = callback => {try {callback();} catch (error) {return error;}};
  const frame = document.querySelector('iframe');
  if (frame.contentDocument.readyState !== 'complete') await new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const other = frame.contentWindow;
  const ns = 'http://www.w3.org/2000/svg';
  const cases = [
    ['feBlend', 'SVGFEBlendElement', ['in1','in2']],
    ['feColorMatrix', 'SVGFEColorMatrixElement', ['in1']],
    ['feComponentTransfer', 'SVGFEComponentTransferElement', ['in1']],
    ['feComposite', 'SVGFECompositeElement', ['in1','in2']],
    ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', ['in1']],
    ['feDiffuseLighting', 'SVGFEDiffuseLightingElement', ['in1']],
    ['feDisplacementMap', 'SVGFEDisplacementMapElement', ['in1','in2']],
    ['feDropShadow', 'SVGFEDropShadowElement', ['in1']],
    ['feFlood', 'SVGFEFloodElement', []],
    ['feGaussianBlur', 'SVGFEGaussianBlurElement', ['in1']],
    ['feImage', 'SVGFEImageElement', []],
    ['feMerge', 'SVGFEMergeElement', []],
    ['feMorphology', 'SVGFEMorphologyElement', ['in1']],
    ['feOffset', 'SVGFEOffsetElement', ['in1']],
    ['feSpecularLighting', 'SVGFESpecularLightingElement', ['in1']],
    ['feTile', 'SVGFETileElement', ['in1']],
    ['feTurbulence', 'SVGFETurbulenceElement', []],
    ['feMergeNode', 'SVGFEMergeNodeElement', ['in1']],
  ];
  const region = ['x','y','width','height'];
  for (const w of [window, other]) {
    const realm = w === window ? 'main' : 'child';
    const documents = [w.document, w.document.implementation.createHTMLDocument(''),
      new w.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
    for (const [tag, name, inputs] of cases) {
      const p = w[name].prototype;
      const strings = [...(tag === 'feMergeNode' ? [] : ['result']), ...inputs];
      const properties = [...(tag === 'feMergeNode' ? [] : region), ...strings];
      for (const property of properties) run(realm + '/' + tag + '/descriptor/' + property, () => {
        const d = Object.getOwnPropertyDescriptor(p, property);
        return d.enumerable && d.configurable && d.set === undefined && d.get.name === 'get ' + property && d.get.length === 0;
      });
      for (const [index, doc] of documents.entries()) {
        const label = realm + '/' + tag + '/document-' + index;
        const element = doc.createElementNS(ns, tag);
        for (const property of properties) {
          const getter = Object.getOwnPropertyDescriptor(p, property)?.get;
          run(label + '/stable-native-value/' + property, () => {
            const value = getter.call(element);
            const expected = strings.includes(property) ? w.SVGAnimatedString : w.SVGAnimatedLength;
            return value === getter.call(element) && Object.getPrototypeOf(value) === expected.prototype && !Object.hasOwn(element, property);
          });
          const revoked = Proxy.revocable(element, {});revoked.revoke();
          let traps = 0;
          const bad = [{}, p, Object.create(element), new Proxy(element, {get() {traps++;throw 42;}}), revoked.proxy,
            doc.createElementNS(ns, tag === 'feFlood' ? 'feBlend' : 'feFlood'), doc.createElement(tag), doc.createElementNS('urn:other', tag)];
          for (const [n, receiver] of bad.entries()) run(label + '/receiver/' + property + '/' + n, () =>
            typeof getter === 'function' && caught(() => getter.call(receiver)) instanceof w.TypeError && traps === 0);
        }
        for (const property of strings) {
          const attribute = property === 'in1' ? 'in' : property;
          run(label + '/string-reflection/' + property, () => {
            const value = element[property];
            if (value.baseVal !== '' || value.animVal !== '') return false;
            element.setAttribute(attribute, 'SourceGraphic');
            if (value.baseVal !== 'SourceGraphic' || value.animVal !== 'SourceGraphic') return false;
            let conversions = 0;
            value.baseVal = {toString() {conversions++;return 'input\ud800output';}};
            if (conversions !== 1 || element.getAttribute(attribute) !== 'input\ud800output' || value.animVal !== 'input\ud800output') return false;
            const sentinel = {};
            if (caught(() => {value.baseVal = {toString() {throw sentinel;}};}) !== sentinel || value.baseVal !== 'input\ud800output') return false;
            if (!(caught(() => {value.baseVal = Symbol();}) instanceof w.TypeError) || value.baseVal !== 'input\ud800output') return false;
            value.animVal = 'ignored';
            if (value.animVal !== 'input\ud800output') return false;
            element.removeAttribute(attribute);
            return value.baseVal === '' && value.animVal === '' && value === element[property];
          });
          run(label + '/native-mutation-observation/' + property, () => {
            const value = element[property], observer = new w.MutationObserver(() => {});
            observer.observe(element, {attributes: true, attributeOldValue: true});
            try {
              value.baseVal = 'observed';
              const records = observer.takeRecords();
              return records.length === 1 && records[0].target === element && records[0].attributeName === attribute && records[0].oldValue === null;
            } finally {observer.disconnect(); element.removeAttribute(attribute);}
          });
          run(label + '/value-brand-conversion-order/' + property, () => {
            const value = element[property], d = Object.getOwnPropertyDescriptor(w.SVGAnimatedString.prototype, 'baseVal');
            let reads = 0, conversions = 0;
            const author = new Proxy(value, {get() {reads++;throw 42;}});
            const error = caught(() => d.set.call(author, {toString() {conversions++;return 'forged';}}));
            return error instanceof w.TypeError && reads === 0 && conversions === 0;
          });
        }
        if (tag !== 'feMergeNode') for (const property of region) run(label + '/region-reflection/' + property, () => {
          const value = element[property], initial = property === 'x' || property === 'y' ? '0%' : '100%';
          if (value.baseVal.valueAsString !== initial || value.animVal.valueAsString !== initial) return false;
          element.setAttribute(property, '12');
          if (value.baseVal.valueInSpecifiedUnits !== 12 || value.animVal.valueInSpecifiedUnits !== 12) return false;
          value.baseVal.valueAsString = '25%';
          if (element.getAttribute(property) !== '25%' || value.animVal.valueAsString !== '25%') return false;
          if (caught(() => {value.animVal.value = 1;})?.name !== 'NoModificationAllowedError') return false;
          element.removeAttribute(property);
          return value.baseVal.valueAsString === initial;
        });
        run(label + '/cache-independent-of-author-properties', () => {
          const property = strings[0], getter = Object.getOwnPropertyDescriptor(p, property).get;
          const value = getter.call(element);
          element.__moliSvgFilterResult = 'forged';element.__moliSvgFilterInput = 'forged';element.__moliSvgFilterInput2 = 'forged';
          Object.defineProperty(element, property, {value: 'shadow', configurable: true});
          Object.setPrototypeOf(element, null);Object.freeze(element);
          return getter.call(element) === value;
        });
      }
      for (const property of properties) run(realm + '/' + tag + '/borrowed-accessor/' + property, () => {
        const owner = w === window ? other : window, element = owner.document.createElementNS(ns, tag);
        const getter = Object.getOwnPropertyDescriptor(p, property).get;
        const value = getter.call(element), expected = strings.includes(property) ? owner.SVGAnimatedString : owner.SVGAnimatedLength;
        return Object.getPrototypeOf(value) === expected.prototype && element[property] === value;
      });
    }
  }
  run('independent-input-output-caches', () => {
    const e = document.createElementNS(ns, 'feBlend'), a = e.in1, b = e.in2, r = e.result;
    a.baseVal = 'first';b.baseVal = 'second';r.baseVal = 'output';
    return a !== b && a !== r && b !== r && a.animVal === 'first' && b.animVal === 'second' && r.animVal === 'output';
  });
  run('retained-filter-values-after-iframe-removal', () => {
    const f = document.createElement('iframe');document.body.appendChild(f);
    const e = f.contentDocument.createElementNS(ns, 'feBlend'), a = e.in1, b = e.in2, r = e.result;
    f.remove();a.baseVal = 'first';b.baseVal = 'second';r.baseVal = 'output';
    return e.getAttribute('in') === 'first' && e.getAttribute('in2') === 'second' && e.getAttribute('result') === 'output';
  });
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed: checks.filter(row => row.passed).length};
  return true;
})()
