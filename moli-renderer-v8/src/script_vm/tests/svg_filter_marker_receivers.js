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
  const definitions = [
    ['filter', 'SVGFilterElement', [
      ['x', 'SVGAnimatedLength'], ['y', 'SVGAnimatedLength'],
      ['width', 'SVGAnimatedLength'], ['height', 'SVGAnimatedLength'],
      ['filterUnits', 'SVGAnimatedEnumeration'], ['primitiveUnits', 'SVGAnimatedEnumeration'],
    ]],
    ['marker', 'SVGMarkerElement', [
      ['refX', 'SVGAnimatedLength'], ['refY', 'SVGAnimatedLength'],
      ['markerWidth', 'SVGAnimatedLength'], ['markerHeight', 'SVGAnimatedLength'],
      ['markerUnits', 'SVGAnimatedEnumeration'], ['orientType', 'SVGAnimatedEnumeration'],
      ['orientAngle', 'SVGAnimatedAngle'],
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
        for (const [property, type] of properties) {
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
          const wrong = doc.createElementNS(ns, tag === 'filter' ? 'marker' : 'filter');
          Object.setPrototypeOf(wrong, proto);
          for (const [label, receiver] of [
            ['undefined', undefined], ['null', null], ['boolean', true], ['number', 42],
            ['symbol', Symbol('receiver')], ['plain', {}], ['interface-prototype', proto],
            ['forged-prototype', Object.create(proto)], ['inherit-real', Object.create(real)],
            ['author-proxy', author], ['revoked-proxy', revoked.proxy],
            ['wrong-svg', doc.createElementNS(ns, tag === 'filter' ? 'marker' : 'filter')],
            ['html-namespace', doc.createElementNS('http://www.w3.org/1999/xhtml', tag)],
            ['prototype-swapped-wrong-svg', wrong],
          ]) {
            check(prefix + '/receiver/' + label, () => typeError(getter, receiver, realm) && traps === 0);
          }
          check(prefix + '/retained-value-after-invalid-calls', () => real[property] === value);
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
