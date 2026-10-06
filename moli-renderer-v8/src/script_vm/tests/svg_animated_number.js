(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const scalar = names => names.split(' ').map(name => [name, name, 0]);
  const pair = (attribute, first, second) => [[first, attribute, 1], [second, attribute, 2]];
  const cases = [
    ...['feFuncR','feFuncG','feFuncB','feFuncA'].map(tag => [tag, 'SVGComponentTransferFunctionElement', scalar('slope intercept amplitude exponent offset')]),
    ['feComposite', 'SVGFECompositeElement', scalar('k1 k2 k3 k4')],
    ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', [...scalar('divisor bias'), ...pair('kernelUnitLength', 'kernelUnitLengthX', 'kernelUnitLengthY')]],
    ['feDiffuseLighting', 'SVGFEDiffuseLightingElement', [...scalar('surfaceScale diffuseConstant'), ...pair('kernelUnitLength', 'kernelUnitLengthX', 'kernelUnitLengthY')]],
    ['feDisplacementMap', 'SVGFEDisplacementMapElement', scalar('scale')],
    ['feDistantLight', 'SVGFEDistantLightElement', scalar('azimuth elevation')],
    ['feDropShadow', 'SVGFEDropShadowElement', [...scalar('dx dy'), ...pair('stdDeviation', 'stdDeviationX', 'stdDeviationY')]],
    ['feGaussianBlur', 'SVGFEGaussianBlurElement', pair('stdDeviation', 'stdDeviationX', 'stdDeviationY')],
    ['feMorphology', 'SVGFEMorphologyElement', pair('radius', 'radiusX', 'radiusY')],
    ['feOffset', 'SVGFEOffsetElement', scalar('dx dy')],
    ['fePointLight', 'SVGFEPointLightElement', scalar('x y z')],
    ['feSpecularLighting', 'SVGFESpecularLightingElement', [...scalar('surfaceScale specularConstant specularExponent'), ...pair('kernelUnitLength', 'kernelUnitLengthX', 'kernelUnitLengthY')]],
    ['feSpotLight', 'SVGFESpotLightElement', scalar('x y z pointsAtX pointsAtY pointsAtZ specularExponent limitingConeAngle')],
    ['feTurbulence', 'SVGFETurbulenceElement', [...pair('baseFrequency', 'baseFrequencyX', 'baseFrequencyY'), ...scalar('seed')]],
    ['stop', 'SVGStopElement', scalar('offset')],
    ['path', 'SVGGeometryElement', [['pathLength', 'pathLength', 0]]],
  ];
  globalThis.__svgNumberDefinitions = cases;
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.querySelector('iframe').contentWindow;
  const invoke = (fn, receiver, args = []) => Reflect.apply(fn, receiver, args);
  const throwsTypeError = (fn, receiver, realm, args = []) => {
    try { invoke(fn, receiver, args); }
    catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const invalid = (real, proto, wrong) => {
    let traps = 0;
    const author = new Proxy(real, {get() {traps++;throw 42;}, getPrototypeOf() {traps++;throw 42;}});
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    Object.setPrototypeOf(wrong, proto);
    return {traps: () => traps, values: [
      ['undefined', undefined], ['null', null], ['boolean', false], ['number', 1],
      ['symbol', Symbol()], ['bigint', 1n], ['plain', {}], ['prototype', proto],
      ['forged', Object.create(proto)], ['inherit-real', Object.create(real)],
      ['author-proxy', author], ['revoked-proxy', revoked.proxy], ['wrong-brand-swapped-prototype', wrong],
    ]};
  };
  const valuesMatch = (value, expected) => Object.is(value.baseVal, expected) && Object.is(value.animVal, expected);
  const readContent = (element, attribute, component) => {
    const values = element.getAttribute(attribute).trim().split(/[\s,]+/).map(Number);
    return Math.fround(values[component === 2 ? 1 : 0]);
  };
  for (const [world, realm] of [['main', globalThis], ['child', child]]) {
    const foreignRealm = realm === globalThis ? child : globalThis;
    for (const [kind, doc] of [
      ['live', realm.document], ['windowless-html', realm.document.implementation.createHTMLDocument('')],
      ['windowless-xml', new realm.DOMParser().parseFromString('<root/>', 'application/xml')],
    ]) {
      for (const [tag, name, members] of cases) for (const [member, attribute, component] of members) {
        const prefix = world + '/' + kind + '/' + tag + '/' + member;
        const make = () => doc.createElementNS(ns, tag);
        const element = make(), proto = realm[name].prototype;
        const descriptor = Object.getOwnPropertyDescriptor(proto, member), getter = descriptor?.get;
        const foreign = Object.getOwnPropertyDescriptor(foreignRealm[name].prototype, member)?.get;
        check(prefix + '/descriptor', () => typeof getter === 'function' && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
        check(prefix + '/same-object', () => invoke(getter, element) === invoke(getter, element) && Object.getPrototypeOf(invoke(getter, element)) === realm.SVGAnimatedNumber.prototype);
        check(prefix + '/foreign-cold-producer-realm', () => {
          const owner = make(), value = invoke(foreign, owner);
          return value === invoke(getter, owner) && Object.getPrototypeOf(value) === realm.SVGAnimatedNumber.prototype;
        });
        check(prefix + '/genuine-swapped-prototype', () => {
          const owner = make(); Object.setPrototypeOf(owner, realm.Object.prototype);
          return Object.getPrototypeOf(invoke(getter, owner)) === realm.SVGAnimatedNumber.prototype;
        });
        check(prefix + '/author-property', () => {
          const owner = make(), value = invoke(getter, owner); let reads = 0;
          Object.defineProperty(owner, member, {get() {reads++;throw 42;}});
          return invoke(getter, owner) === value && reads === 0;
        });
        const badOwners = invalid(element, proto, doc.createElementNS(ns, 'svg'));
        if (name === 'SVGGeometryElement') check(prefix + '/genuine-geometry-subtype', () => {
          const owner = doc.createElementNS(ns, 'rect'); Object.setPrototypeOf(owner, proto);
          return Object.getPrototypeOf(invoke(getter, owner)) === realm.SVGAnimatedNumber.prototype;
        });
        for (const [label, receiver] of badOwners.values) check(prefix + '/owner/' + label, () => throwsTypeError(getter, receiver, realm) && badOwners.traps() === 0);
        check(prefix + '/foreign-owner-error-realm', () => throwsTypeError(foreign, {}, foreignRealm));
        const value = invoke(getter, element), initial = value.baseVal;
        const valueProto = realm.SVGAnimatedNumber.prototype;
        const base = Object.getOwnPropertyDescriptor(valueProto, 'baseVal');
        const anim = Object.getOwnPropertyDescriptor(valueProto, 'animVal');
        const badValues = invalid(value, valueProto, doc.createElementNS(ns, 'svg').createSVGNumber());
        for (const [label, receiver] of badValues.values) {
          check(prefix + '/base-getter/' + label, () => throwsTypeError(base.get, receiver, realm) && badValues.traps() === 0);
          check(prefix + '/anim-getter/' + label, () => throwsTypeError(anim.get, receiver, realm) && badValues.traps() === 0);
          check(prefix + '/setter-before-conversion/' + label, () => {
            let conversions = 0;
            return throwsTypeError(base.set, receiver, realm, [{valueOf() {conversions++;throw 42;}}]) && conversions === 0 && badValues.traps() === 0;
          });
        }
        const raw = component ? '4.25 7.5' : '0.1';
        const expected = component === 1 ? 4.25 : component === 2 ? 7.5 : Math.fround(0.1);
        check(prefix + '/held-value-float-reflection', () => {
          element.setAttribute(attribute, raw);
          return valuesMatch(value, expected) && value === invoke(getter, element);
        });
        check(prefix + '/clone-import', () => {
          const clone = element.cloneNode(false), imported = doc.importNode(element, false);
          return valuesMatch(invoke(getter, clone), expected) && valuesMatch(invoke(getter, imported), expected) && invoke(getter, clone) !== value;
        });
        check(prefix + '/single-conversion-writeback', () => {
          let conversions = 0;
          invoke(base.set, value, [{valueOf() {conversions++;return 0.75;}}]);
          return conversions === 1 && valuesMatch(value, 0.75) && readContent(element, attribute, component) === 0.75;
        });
        check(prefix + '/conversion-exception-preserves-state', () => {
          const sentinel = {}, before = element.getAttribute(attribute); let error, conversions = 0;
          try {invoke(base.set, value, [{valueOf() {conversions++;throw sentinel;}}]);} catch (caught) {error = caught;}
          return error === sentinel && conversions === 1 && element.getAttribute(attribute) === before && valuesMatch(value, 0.75);
        });
        for (const [label, input] of [['decimal', 0.1], ['rounded-integer', 16777217], ['maximum-float', Math.fround(3.4028234663852886e38)], ['subnormal', Math.fround(1e-45)], ['negative-zero', -0], ['underflow', 1e-99], ['negative-underflow', -1e-99]]) {
          check(prefix + '/float/' + label, () => {
            invoke(base.set, value, [input]);
            return valuesMatch(value, Math.fround(input)) && Object.is(readContent(element, attribute, component), Math.fround(input));
          });
        }
        for (const [label, input] of [['overflow', 1e40], ['negative-overflow', -1e40], ['nan', NaN], ['infinity', Infinity], ['symbol', Symbol()], ['bigint', 1n]]) {
          check(prefix + '/reject-float/' + label, () => {
            const before = element.getAttribute(attribute), previous = value.baseVal;
            return throwsTypeError(base.set, value, realm, [input]) && element.getAttribute(attribute) === before && valuesMatch(value, previous);
          });
        }
        if (tag === 'stop') check(prefix + '/percentage', () => {
          element.setAttribute(attribute, '25%'); return valuesMatch(value, 0.25);
        });
        if (component) check(prefix + '/pair-preserves-other-component', () => {
          element.setAttribute(attribute, raw); invoke(base.set, value, [0.75]);
          const parts = element.getAttribute(attribute).trim().split(/\s+/).map(Number);
          return parts.length === 2 && parts[component === 1 ? 1 : 0] === (component === 1 ? 7.5 : 4.25);
        });
        for (const [label, raw] of [['overflow', component ? '4.25 1e40' : '1e40'], ['syntax', 'invalid']]) {
          check(prefix + '/invalid-content/' + label, () => {
            element.setAttribute(attribute, raw); return valuesMatch(value, initial);
          });
        }
        check(prefix + '/reset-retains-wrapper', () => {
          element.removeAttribute(attribute); return valuesMatch(value, initial) && value === invoke(getter, element);
        });
        check(prefix + '/foreign-value-setter-error-realm', () => {
          const setter = Object.getOwnPropertyDescriptor(foreignRealm.SVGAnimatedNumber.prototype, 'baseVal').set;
          let conversions = 0;
          return throwsTypeError(setter, {}, foreignRealm, [{valueOf() {conversions++;throw 42;}}]) && conversions === 0;
        });
      }
    }
  }
  if (globalThis.__nativeSvgNumberOwners) for (const entry of __nativeSvgNumberOwners) {
    const {tag, name, member, attribute, component, target, proxy} = entry;
    const prefix = 'native-owner/' + tag + '/' + member;
    const getter = Object.getOwnPropertyDescriptor(globalThis[name].prototype, member).get;
    let number;
    check(prefix + '/cold-normalized-target-and-producer-realm', () => {
      number = invoke(getter, proxy);
      return number === invoke(getter, target) && Object.getPrototypeOf(number) === child.SVGAnimatedNumber.prototype;
    });
    check(prefix + '/writeback', () => {
      number.baseVal = 0.75; return valuesMatch(number, 0.75) && readContent(target, attribute, component) === 0.75;
    });
    const bad = invalid(proxy, globalThis[name].prototype, {});
    for (const label of ['author-proxy','revoked-proxy','inherit-real']) {
      const receiver = bad.values.find(row => row[0] === label)[1];
      check(prefix + '/' + label, () => throwsTypeError(getter, receiver, globalThis) && bad.traps() === 0);
    }
  }
  if (globalThis.__nativeSvgNumberValues) for (const entry of __nativeSvgNumberValues) {
    const {tag, member, attribute, component, owner, target, proxy} = entry;
    const prefix = 'native-value/' + tag + '/' + member;
    const base = Object.getOwnPropertyDescriptor(SVGAnimatedNumber.prototype, 'baseVal');
    const anim = Object.getOwnPropertyDescriptor(SVGAnimatedNumber.prototype, 'animVal');
    check(prefix + '/getters', () => invoke(base.get, proxy) === invoke(base.get, target) && invoke(anim.get, proxy) === invoke(anim.get, target));
    check(prefix + '/writeback', () => {
      invoke(base.set, proxy, [0.75]); return valuesMatch(target, 0.75) && readContent(owner, attribute, component) === 0.75;
    });
    check(prefix + '/float-conversion-and-state', () => {
      invoke(base.set, proxy, [0.1]); return valuesMatch(target, Math.fround(0.1)) && invoke(base.get, proxy) === Math.fround(0.1);
    });
    const bad = invalid(proxy, SVGAnimatedNumber.prototype, {});
    for (const label of ['author-proxy','revoked-proxy','inherit-real']) {
      const receiver = bad.values.find(row => row[0] === label)[1];
      check(prefix + '/' + label, () => {
        let conversions = 0;
        return throwsTypeError(base.set, receiver, globalThis, [{valueOf() {conversions++;throw 42;}}]) && conversions === 0 && bad.traps() === 0;
      });
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
