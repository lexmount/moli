(() => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const ns = 'http://www.w3.org/2000/svg';
  const realms = [window, document.querySelector('iframe').contentWindow];
  const edgeConstants = [['SVG_EDGEMODE_UNKNOWN', 0], ['SVG_EDGEMODE_DUPLICATE', 1],
    ['SVG_EDGEMODE_WRAP', 2], ['SVG_EDGEMODE_NONE', 3]];
  const cases = [
    ['feComposite', 'SVGFECompositeElement', 'operator', 1,
      [['over', 1], ['in', 2], ['out', 3], ['atop', 4], ['xor', 5], ['arithmetic', 6], ['lighter', 7]],
      [['SVG_FECOMPOSITE_OPERATOR_UNKNOWN', 0], ['SVG_FECOMPOSITE_OPERATOR_OVER', 1],
       ['SVG_FECOMPOSITE_OPERATOR_IN', 2], ['SVG_FECOMPOSITE_OPERATOR_OUT', 3],
       ['SVG_FECOMPOSITE_OPERATOR_ATOP', 4], ['SVG_FECOMPOSITE_OPERATOR_XOR', 5],
       ['SVG_FECOMPOSITE_OPERATOR_ARITHMETIC', 6]]],
    ['feGaussianBlur', 'SVGFEGaussianBlurElement', 'edgeMode', 3,
      [['duplicate', 1], ['wrap', 2], ['none', 3]], edgeConstants],
    ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', 'edgeMode', 1,
      [['duplicate', 1], ['wrap', 2], ['none', 3]], edgeConstants],
    ['feMorphology', 'SVGFEMorphologyElement', 'operator', 1,
      [['erode', 1], ['dilate', 2]], [['SVG_MORPHOLOGY_OPERATOR_UNKNOWN', 0],
       ['SVG_MORPHOLOGY_OPERATOR_ERODE', 1], ['SVG_MORPHOLOGY_OPERATOR_DILATE', 2]]],
    ['feDisplacementMap', 'SVGFEDisplacementMapElement', 'xChannelSelector', 4,
      [['R', 1], ['G', 2], ['B', 3], ['A', 4]], [['SVG_CHANNEL_UNKNOWN', 0],
       ['SVG_CHANNEL_R', 1], ['SVG_CHANNEL_G', 2], ['SVG_CHANNEL_B', 3], ['SVG_CHANNEL_A', 4]]],
  ];
  const throwsTypeError = (realm, fn) => {
    try { fn(); } catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
    return false;
  };
  for (const [ownerIndex, owner] of realms.entries()) {
    for (const [calleeIndex, realm] of realms.entries()) {
      const documents = [owner.document, owner.document.implementation.createHTMLDocument(''),
        owner.document.implementation.createDocument(ns, 'svg'),
        new owner.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
      for (const [documentIndex, doc] of documents.entries()) {
        for (const [tag, iface, property, initial, keywords, constants] of cases) {
          const prefix = `${ownerIndex}/${calleeIndex}/${documentIndex}/${tag}/${property}`;
          const element = doc.createElementNS(ns, tag);
          (doc.body || doc.documentElement).appendChild(element);
          const ctor = realm[iface];
          const getter = Object.getOwnPropertyDescriptor(ctor.prototype, property)?.get;
          const enumProto = realm.SVGAnimatedEnumeration.prototype;
          const base = Object.getOwnPropertyDescriptor(enumProto, 'baseVal');
          const anim = Object.getOwnPropertyDescriptor(enumProto, 'animVal');
          const value = element[property];
          check(prefix + '/attribute-descriptor', () => {
            const descriptor = Object.getOwnPropertyDescriptor(ctor.prototype, property);
            return descriptor.enumerable && descriptor.configurable && descriptor.set === undefined &&
              descriptor.get.length === 0 && typeof descriptor.get === 'function';
          });
          check(prefix + '/enum-descriptors', () => base.enumerable && base.configurable &&
            base.get.length === 0 && base.set.length === 1 && anim.enumerable && anim.configurable &&
            anim.get.length === 0 && anim.set === undefined);
          check(prefix + '/owner-realm-and-identity', () =>
            Object.getPrototypeOf(value) === owner.SVGAnimatedEnumeration.prototype &&
            getter.call(element) === value && element[property] === value);
          check(prefix + '/missing-initial', () => value.baseVal === initial && value.animVal === initial &&
            !element.hasAttribute(property));
          for (const [name, expected] of constants) {
            for (const [targetName, target] of [['constructor', ctor], ['prototype', ctor.prototype]]) {
              check(prefix + '/constant/' + targetName + '/' + name, () => {
                const descriptor = Object.getOwnPropertyDescriptor(target, name);
                return descriptor.value === expected && descriptor.enumerable &&
                  !descriptor.writable && !descriptor.configurable;
              });
            }
          }
          for (const [keyword, expected] of keywords) {
            check(prefix + '/attribute-to-enum/' + keyword, () => {
              element.setAttribute(property, keyword);
              return base.get.call(value) === expected && anim.get.call(value) === expected &&
                getter.call(element) === value;
            });
            check(prefix + '/enum-to-attribute/' + keyword, () => {
              base.set.call(value, expected);
              return element.getAttribute(property) === keyword && value.baseVal === expected &&
                value.animVal === expected;
            });
          }
          for (const invalid of ['', 'invalid', 'NONE', ' duplicate ', '\u0000']) {
            check(prefix + '/invalid-content-initial/' + JSON.stringify(invalid), () => {
              element.setAttribute(property, invalid);
              return value.baseVal === initial && value.animVal === initial &&
                element.getAttribute(property) === invalid;
            });
          }
          check(prefix + '/remove-initial-and-identity', () => {
            element.removeAttribute(property);
            return value.baseVal === initial && value.animVal === initial && getter.call(element) === value;
          });
          const [keyword, expected] = keywords[0];
          for (const [name, argument] of [['wrapped', 65536 + expected], ['negative-wrapped', expected - 65536],
            ['fractional', expected + 0.75], ['string', String(expected)]]) {
            check(prefix + '/unsigned-short/' + name, () => {
              base.set.call(value, argument);
              return value.baseVal === expected && element.getAttribute(property) === keyword;
            });
          }
          for (const [name, argument] of [['zero', 0], ['outside-domain', 42], ['nan', NaN],
            ['infinite', Infinity], ['undefined', undefined], ['null', null], ['negative', -1],
            ['symbol', Symbol('enum')], ['bigint', 1n]]) {
            check(prefix + '/invalid-setter/' + name, () => {
              element.setAttribute(property, keyword);
              return throwsTypeError(realm, () => base.set.call(value, argument)) &&
                element.getAttribute(property) === keyword && value.baseVal === expected;
            });
          }
          check(prefix + '/conversion-once-and-live-mutation', () => {
            let conversions = 0;
            base.set.call(value, {valueOf() { conversions++; element.setAttribute(property, 'invalid'); return expected; }});
            return conversions === 1 && element.getAttribute(property) === keyword && value.baseVal === expected;
          });
          check(prefix + '/original-conversion-exception', () => {
            const sentinel = {}; let conversions = 0;
            try { base.set.call(value, {valueOf() { conversions++; throw sentinel; }}); }
            catch (error) { return error === sentinel && conversions === 1 && element.getAttribute(property) === keyword; }
            return false;
          });
          check(prefix + '/clone-independent-identity', () => {
            const clone = element.cloneNode();
            const cloned = getter.call(clone);
            clone.setAttribute(property, keywords[1][0]);
            return cloned !== value && cloned.baseVal === keywords[1][1] &&
              value.baseVal === expected && Object.getPrototypeOf(cloned) === owner.SVGAnimatedEnumeration.prototype;
          });
          check(prefix + '/retained-value-after-detach', () => {
            element.remove(); element.setAttribute(property, keywords[1][0]);
            return getter.call(element) === value && value.baseVal === keywords[1][1];
          });
          const wrongElement = doc.createElementNS(ns, tag === 'feMorphology' ? 'feConvolveMatrix' : 'feMorphology');
          for (const [index, receiver] of [wrongElement, doc.createElement('div'), {}, Object.create(element)].entries()) {
            check(prefix + '/element-receiver/' + index, () => throwsTypeError(realm, () => getter.call(receiver)));
          }
          let traps = 0;
          const authorProxy = new Proxy(value || {}, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
          const revoked = Proxy.revocable(value || {}, {}); revoked.revoke();
          const invalidReceivers = [{}, Object.create(enumProto), Object.create(value || enumProto),
            authorProxy, revoked.proxy, element, doc, doc.createElementNS(ns, 'circle')];
          for (const [index, receiver] of invalidReceivers.entries()) {
            for (const [name, fn] of [['base', base.get], ['anim', anim.get]]) {
              check(prefix + '/enum-receiver/' + index + '/' + name, () =>
                throwsTypeError(realm, () => fn.call(receiver)) && traps === 0);
            }
            check(prefix + '/enum-receiver/' + index + '/set-before-conversion', () => {
              let conversions = 0;
              return throwsTypeError(realm, () => base.set.call(receiver, {valueOf() {conversions++; return expected;}})) &&
                conversions === 0 && traps === 0;
            });
          }
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
