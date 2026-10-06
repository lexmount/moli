(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const fields = (names, type) => names.split(' ').map(name => [name, type]);
  const definitions = [
    ['text', 'SVGTextContentElement', [['textLength', 'SVGAnimatedLength'], ['lengthAdjust', 'SVGAnimatedEnumeration']]],
    ['tspan', 'SVGTextContentElement', [['textLength', 'SVGAnimatedLength'], ['lengthAdjust', 'SVGAnimatedEnumeration']]],
    ['textPath', 'SVGTextContentElement', [['textLength', 'SVGAnimatedLength'], ['lengthAdjust', 'SVGAnimatedEnumeration']]],
    ['textPath', 'SVGTextPathElement', [['startOffset', 'SVGAnimatedLength'], ...fields('method spacing side', 'SVGAnimatedEnumeration')]],
    ['pattern', 'SVGPatternElement', [...fields('x y width height', 'SVGAnimatedLength'), ...fields('patternUnits patternContentUnits', 'SVGAnimatedEnumeration'), ['patternTransform', 'SVGAnimatedTransformList']]],
    ['linearGradient', 'SVGGradientElement', [...fields('gradientUnits spreadMethod', 'SVGAnimatedEnumeration'), ['gradientTransform', 'SVGAnimatedTransformList']]],
    ['radialGradient', 'SVGGradientElement', [...fields('gradientUnits spreadMethod', 'SVGAnimatedEnumeration'), ['gradientTransform', 'SVGAnimatedTransformList']]],
    ['linearGradient', 'SVGLinearGradientElement', fields('x1 y1 x2 y2', 'SVGAnimatedLength')],
    ['radialGradient', 'SVGRadialGradientElement', fields('cx cy r fx fy fr', 'SVGAnimatedLength')],
    ['mask', 'SVGMaskElement', [...fields('x y width height', 'SVGAnimatedLength'), ...fields('maskUnits maskContentUnits', 'SVGAnimatedEnumeration')]],
    ['clipPath', 'SVGClipPathElement', [['clipPathUnits', 'SVGAnimatedEnumeration']]],
    ['image', 'SVGImageElement', [['preserveAspectRatio', 'SVGAnimatedPreserveAspectRatio']]],
    ['a', 'SVGAElement', [['relList', 'DOMTokenList']]],
  ];
  const methods = [
    'getNumberOfChars', 'getComputedTextLength', 'getSubStringLength',
    'getStartPositionOfChar', 'getEndPositionOfChar', 'getExtentOfChar',
    'getRotationOfChar', 'getCharNumAtPosition', 'selectSubString',
  ];
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.querySelector('iframe').contentWindow;
  const invoke = (fn, receiver, args = []) => Reflect.apply(fn, receiver, args);
  const throwsTypeError = (fn, receiver, realm, args = []) => {
    if (typeof fn !== 'function') return false;
    try { invoke(fn, receiver, args); }
    catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const invalidReceivers = (realm, doc, real, proto) => {
    let traps = 0;
    const author = new Proxy(real, {
      get() { traps++; throw 42; },
      getPrototypeOf() { traps++; throw 42; },
      has() { traps++; throw 42; },
    });
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    const wrong = doc.createElementNS(ns, 'rect');
    Object.setPrototypeOf(wrong, proto);
    return {
      traps: () => traps,
      values: [
        ['undefined', undefined], ['null', null], ['boolean', true], ['number', 42],
        ['symbol', Symbol('receiver')], ['bigint', 1n], ['plain', {}],
        ['interface-prototype', proto], ['forged-prototype', Object.create(proto)],
        ['inherit-real', Object.create(real)], ['author-proxy', author], ['revoked-proxy', revoked.proxy],
        ['wrong-svg', doc.createElementNS(ns, 'rect')], ['html-namespace', doc.createElement('a')],
        ['text-node', doc.createTextNode('')], ['document', doc], ['prototype-swapped-wrong-svg', wrong],
      ],
    };
  };
  for (const [world, realm] of [['main', globalThis], ['child', child]]) {
    const foreignRealm = realm === globalThis ? child : globalThis;
    for (const [kind, doc] of [
      ['live', realm.document],
      ['windowless-html', realm.document.implementation.createHTMLDocument('')],
      ['windowless-xml', new realm.DOMParser().parseFromString('<root/>', 'application/xml')],
    ]) {
      for (const [tag, name, members] of definitions) {
        for (const [member, type] of members) {
          const prefix = world + '/' + kind + '/' + tag + '/' + name + '.' + member;
          const make = () => doc.createElementNS(ns, tag);
          const real = make(), proto = realm[name].prototype;
          const descriptor = Object.getOwnPropertyDescriptor(proto, member);
          const getter = descriptor?.get;
          const foreign = Object.getOwnPropertyDescriptor(foreignRealm[name].prototype, member)?.get;
          check(prefix + '/declaring-prototype', () => typeof getter === 'function');
          check(prefix + '/descriptor', () => descriptor.enumerable && descriptor.configurable &&
            (member === 'relList' ? typeof descriptor.set === 'function' : descriptor.set === undefined));
          check(prefix + '/callee-function-realm', () => Object.getPrototypeOf(getter) === realm.Function.prototype);
          check(prefix + '/valid-receiver', () => invoke(getter, real) instanceof realm[type]);
          check(prefix + '/stable-cache', () => invoke(getter, real) === invoke(getter, real));
          check(prefix + '/foreign-getter-cache', () => invoke(foreign, real) === invoke(getter, real));
          check(prefix + '/foreign-getter-cold-producer-realm', () => {
            const element = make(), value = invoke(foreign, element);
            return value === invoke(getter, element) && Object.getPrototypeOf(value) === realm[type].prototype;
          });
          check(prefix + '/clone', () => invoke(getter, real.cloneNode(false)) instanceof realm[type]);
          check(prefix + '/import', () => invoke(getter, doc.importNode(real, false)) instanceof realm[type]);
          check(prefix + '/genuine-prototype-replacement', () => {
            const element = make(); Object.setPrototypeOf(element, realm.Object.prototype);
            return invoke(getter, element) instanceof realm[type];
          });
          check(prefix + '/bypass-author-property', () => {
            const element = make(), original = invoke(getter, element); let reads = 0;
            Object.defineProperty(element, member, {get() { reads++; throw 42; }});
            return invoke(getter, element) === original && reads === 0;
          });
          const invalid = invalidReceivers(realm, doc, real, proto);
          for (const [label, receiver] of invalid.values) {
            check(prefix + '/receiver/' + label, () => throwsTypeError(getter, receiver, realm) && invalid.traps() === 0);
          }
          check(prefix + '/foreign-error-realm', () => throwsTypeError(foreign, {}, foreignRealm));
          if (member === 'relList') {
            const setter = descriptor?.set;
            for (const [label, receiver] of invalid.values) {
              check(prefix + '/setter-before-conversion/' + label, () => {
                let conversions = 0;
                const value = {toString() { conversions++; throw 42; }};
                return throwsTypeError(setter, receiver, realm, [value]) &&
                  conversions === 0 && invalid.traps() === 0 && real.getAttribute('rel') === null;
              });
            }
            check(prefix + '/put-forwards-single-conversion', () => {
              let conversions = 0; const original = invoke(getter, real);
              invoke(setter, real, [{toString() { conversions++; return 'noopener noreferrer'; }}]);
              return conversions === 1 && original === invoke(getter, real) &&
                original.value === 'noopener noreferrer' && real.getAttribute('rel') === 'noopener noreferrer';
            });
            check(prefix + '/conversion-exception', () => {
              const sentinel = {}; let caught;
              try { invoke(setter, real, [{toString() { throw sentinel; }}]); } catch (error) { caught = error; }
              return caught === sentinel && real.getAttribute('rel') === 'noopener noreferrer';
            });
            const foreignSetter = Object.getOwnPropertyDescriptor(foreignRealm[name].prototype, member)?.set;
            check(prefix + '/foreign-setter-error-realm', () => {
              let conversions = 0;
              return throwsTypeError(foreignSetter, {}, foreignRealm, [{toString() { conversions++; return ''; }}]) &&
                conversions === 0;
            });
          }
        }
      }
      for (const tag of ['text', 'tspan', 'textPath']) {
        const real = doc.createElementNS(ns, tag), proto = realm.SVGTextContentElement.prototype;
        for (const member of methods) {
          const prefix = world + '/' + kind + '/' + tag + '/method/' + member;
          const method = proto[member], foreign = foreignRealm.SVGTextContentElement.prototype[member];
          check(prefix + '/declaring-prototype', () => typeof method === 'function' && Object.hasOwn(proto, member));
          check(prefix + '/callee-function-realm', () => Object.getPrototypeOf(method) === realm.Function.prototype);
          const invalid = invalidReceivers(realm, doc, real, proto);
          for (const [label, receiver] of invalid.values) {
            check(prefix + '/receiver-before-conversion/' + label, () => {
              let conversions = 0;
              const value = {valueOf() { conversions++; throw 42; }, get x() { conversions++; throw 42; }};
              return throwsTypeError(method, receiver, realm, [value, value]) &&
                conversions === 0 && invalid.traps() === 0;
            });
          }
          check(prefix + '/foreign-error-realm', () => throwsTypeError(foreign, {}, foreignRealm, [0, 0]));
          if (member === 'getNumberOfChars') {
            check(prefix + '/valid-cross-realm', () => typeof invoke(foreign, real) === 'number');
          } else if (member !== 'getComputedTextLength') {
            check(prefix + '/genuine-conversion-exception', () => {
              const sentinel = {}; let caught, conversions = 0;
              const value = {valueOf() { conversions++; throw sentinel; }, get x() { conversions++; throw sentinel; }};
              try { invoke(method, real, [value, value]); } catch (error) { caught = error; }
              return caught === sentinel && conversions === 1;
            });
          }
        }
      }
    }
  }
  if (globalThis.__nativeSvgReceivers) {
    for (const [tag, name, members] of definitions) {
      const {target, proxy} = __nativeSvgReceivers[tag];
      for (const [member, type] of members) {
        const prefix = 'native-proxy/' + tag + '/' + name + '.' + member;
        const descriptor = Object.getOwnPropertyDescriptor(globalThis[name].prototype, member);
        const getter = descriptor.get;
        check(prefix + '/shared-target-cache-and-realm', () => {
          const value = invoke(getter, proxy);
          return value === invoke(getter, target) && Object.getPrototypeOf(value) === child[type].prototype;
        });
        let traps = 0;
        const author = new Proxy(proxy, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const revoked = Proxy.revocable(proxy, {}); revoked.revoke();
        for (const receiver of [author, revoked.proxy, Object.create(proxy)]) {
          check(prefix + '/reject-layered-author-proxy-' + checks.length,
            () => throwsTypeError(getter, receiver, globalThis) && traps === 0);
        }
        if (member === 'relList') {
          check(prefix + '/put-forwards', () => {
            invoke(descriptor.set, proxy, ['noopener']);
            return target.getAttribute('rel') === 'noopener' && invoke(getter, proxy).value === 'noopener';
          });
          check(prefix + '/author-setter-before-conversion', () => {
            let conversions = 0;
            return throwsTypeError(descriptor.set, author, globalThis, [{toString() { conversions++; return ''; }}]) &&
              conversions === 0 && traps === 0 && target.getAttribute('rel') === 'noopener';
          });
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
