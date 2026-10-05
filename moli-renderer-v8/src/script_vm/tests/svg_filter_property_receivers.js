(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const invoke = (getter, receiver) => Reflect.apply(getter, receiver, []);
  const typeError = (getter, receiver, realm) => {
    try { invoke(getter, receiver); }
    catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.querySelector('iframe').contentWindow;
  const transfer = [['type', 'SVGAnimatedEnumeration', 'type', 'gamma', 5, 4, 'linear']];
  const definitions = [
    ...['feFuncA','feFuncB','feFuncG','feFuncR'].map(tag => [tag, 'SVGComponentTransferFunctionElement', transfer]),
    ['feBlend', 'SVGFEBlendElement', [['mode', 'SVGAnimatedEnumeration', 'mode', 'screen', 3, 2, 'multiply']]],
    ['feColorMatrix', 'SVGFEColorMatrixElement', [['type', 'SVGAnimatedEnumeration', 'type', 'hueRotate', 3, 2, 'saturate']]],
    ['feComposite', 'SVGFECompositeElement', [['operator', 'SVGAnimatedEnumeration', 'operator', 'xor', 5, 2, 'in']]],
    ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', [
      ['orderX', 'SVGAnimatedInteger', 'order', '5 6', 5, 7, '7 6'],
      ['orderY', 'SVGAnimatedInteger', 'order', '5 6', 6, 7, '5 7'],
      ['targetX', 'SVGAnimatedInteger', 'targetX', '2', 2, 3, '3'],
      ['targetY', 'SVGAnimatedInteger', 'targetY', '2', 2, 3, '3'],
      ['edgeMode', 'SVGAnimatedEnumeration', 'edgeMode', 'none', 3, 2, 'wrap'],
      ['preserveAlpha', 'SVGAnimatedBoolean', 'preserveAlpha', 'true', true, false, 'false'],
    ]],
    ['feDisplacementMap', 'SVGFEDisplacementMapElement', [
      ['xChannelSelector', 'SVGAnimatedEnumeration', 'xChannelSelector', 'B', 3, 1, 'R'],
      ['yChannelSelector', 'SVGAnimatedEnumeration', 'yChannelSelector', 'G', 2, 1, 'R'],
    ]],
    ['feMorphology', 'SVGFEMorphologyElement', [['operator', 'SVGAnimatedEnumeration', 'operator', 'dilate', 2, 1, 'erode']]],
    ['feTurbulence', 'SVGFETurbulenceElement', [
      ['numOctaves', 'SVGAnimatedInteger', 'numOctaves', '2', 2, 3, '3'],
      ['stitchTiles', 'SVGAnimatedEnumeration', 'stitchTiles', 'stitch', 1, 2, 'noStitch'],
      ['type', 'SVGAnimatedEnumeration', 'type', 'fractalNoise', 1, 2, 'turbulence'],
    ]],
  ];
  for (const [world, realm] of [['main', globalThis], ['child', child]]) {
    for (const [documentKind, doc] of [
      ['live', realm.document],
      ['windowless-html', realm.document.implementation.createHTMLDocument('')],
      ['windowless-xml', new realm.DOMParser().parseFromString('<root/>', 'application/xml')],
    ]) {
      for (const [tag, name, properties] of definitions) {
        const proto = realm[name].prototype;
        const make = () => doc.createElementNS(ns, tag);
        for (const [property, type, attribute, raw, expected, written, serialized] of properties) {
          const prefix = `${world}/${documentKind}/${tag}/${property}`;
          const descriptor = Object.getOwnPropertyDescriptor(proto, property);
          const getter = descriptor.get;
          const real = make();
          const value = real[property];
          check(prefix + '/readonly-descriptor', () => typeof getter === 'function' &&
            descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
          check(prefix + '/function-realm', () => Object.getPrototypeOf(getter) === realm.Function.prototype);
          check(prefix + '/valid-native-receiver', () => invoke(getter, real) === value && value instanceof realm[type]);
          check(prefix + '/stable-tear-off', () => invoke(getter, real) === invoke(getter, real));
          check(prefix + '/clone', () => invoke(getter, real.cloneNode(false)) instanceof realm[type]);
          check(prefix + '/import', () => invoke(getter, doc.importNode(real, false)) instanceof realm[type]);
          check(prefix + '/bypass-author-property', () => {
            const element = make(), original = element[property]; let reads = 0;
            Object.defineProperty(element, property, {get() { reads++; throw 42; }});
            return invoke(getter, element) === original && reads === 0;
          });
          const other = realm === globalThis ? child : globalThis;
          const foreign = Object.getOwnPropertyDescriptor(other[name].prototype, property).get;
          check(prefix + '/borrowed-foreign-getter', () => invoke(foreign, real) === value);
          check(prefix + '/foreign-error-realm', () => typeError(foreign, {}, other));
          const revoked = Proxy.revocable(real, {}); revoked.revoke();
          let traps = 0;
          const author = new Proxy(real, {
            get() { traps++; throw 42; },
            getPrototypeOf() { traps++; throw 42; },
            has() { traps++; throw 42; },
          });
          const wrong = doc.createElementNS(ns, tag === 'feBlend' ? 'feConvolveMatrix' : 'feBlend');
          Object.setPrototypeOf(wrong, proto);
          for (const [label, receiver] of [
            ['undefined', undefined], ['null', null], ['boolean', true], ['number', 42],
            ['symbol', Symbol('receiver')], ['plain', {}], ['interface-prototype', proto],
            ['forged-prototype', Object.create(proto)], ['inherit-real', Object.create(real)],
            ['author-proxy', author], ['revoked-proxy', revoked.proxy],
            ['wrong-svg', doc.createElementNS(ns, tag === 'feBlend' ? 'feConvolveMatrix' : 'feBlend')],
            ['html-namespace', doc.createElementNS('http://www.w3.org/1999/xhtml', tag)],
            ['prototype-swapped-wrong-svg', wrong],
          ]) {
            check(prefix + '/receiver/' + label, () => typeError(getter, receiver, realm) && traps === 0);
          }
          check(prefix + '/retained-value-after-invalid-calls', () => real[property] === value);
          check(prefix + '/attribute-forward-reflection', () => {
            const element = make(), original = element[property];
            element.setAttribute(attribute, raw);
            return original === element[property] && original.baseVal === expected && original.animVal === expected;
          });
          check(prefix + '/baseVal-writeback', () => {
            const element = make(); element.setAttribute(attribute, raw);
            const original = element[property]; original.baseVal = written;
            return original === element[property] && original.baseVal === written && original.animVal === written &&
              element.getAttribute(attribute) === serialized;
          });
          check(prefix + '/single-native-attribute-mutation', () => {
            const element = make(); element.setAttribute(attribute, raw);
            const original = element[property];
            const observer = new realm.MutationObserver(() => {});
            observer.observe(element, {attributes: true, attributeOldValue: true});
            try {
              original.baseVal = written;
              const records = observer.takeRecords();
              return records.length === 1 && records[0].target === element && records[0].attributeName === attribute &&
                records[0].attributeNamespace === null && records[0].oldValue === raw;
            } finally { observer.disconnect(); }
          });
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
