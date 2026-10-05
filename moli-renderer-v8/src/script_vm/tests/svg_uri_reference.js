(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const invoke = (getter, receiver) => Reflect.apply(getter, receiver, []);
  const typeError = (getter, receiver, realm) => {
    if (typeof getter !== 'function') return false;
    try { invoke(getter, receiver); }
    catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const ns = 'http://www.w3.org/2000/svg';
  const xlink = 'http://www.w3.org/1999/xlink';
  const child = document.querySelector('iframe').contentWindow;
  const definitions = [
    ['a', 'SVGAElement', 'SVGAElement'],
    ['image', 'SVGImageElement', 'SVGImageElement'],
    ['use', 'SVGUseElement', 'SVGUseElement'],
    ['textPath', 'SVGTextPathElement', 'SVGTextPathElement'],
    ['pattern', 'SVGPatternElement', 'SVGPatternElement'],
    ['script', 'SVGScriptElement', 'SVGScriptElement'],
    ['linearGradient', 'SVGGradientElement', 'SVGLinearGradientElement'],
    ['radialGradient', 'SVGGradientElement', 'SVGRadialGradientElement'],
    ['filter', 'SVGFilterElement', 'SVGFilterElement'],
    ['feImage', 'SVGFEImageElement', 'SVGFEImageElement'],
    ['mpath', 'SVGMPathElement', 'SVGMPathElement'],
  ];
  for (const [world, realm] of [['main', globalThis], ['child', child]]) {
    for (const [documentKind, doc] of [
      ['live', realm.document],
      ['windowless-html', realm.document.implementation.createHTMLDocument('')],
      ['windowless-xml', new realm.DOMParser().parseFromString('<root/>', 'application/xml')],
    ]) {
      for (const [tag, name, concrete] of definitions) {
        const prefix = `${world}/${documentKind}/${tag}/href`;
        const proto = realm[name].prototype;
        const descriptor = Object.getOwnPropertyDescriptor(proto, 'href');
        const getter = descriptor?.get;
        const make = () => doc.createElementNS(ns, tag);
        const real = make(), value = real.href;
        const other = realm === globalThis ? child : globalThis;
        const foreign = Object.getOwnPropertyDescriptor(other[name].prototype, 'href')?.get;
        const same = (animated, expected) => animated.baseVal === expected && animated.animVal === expected;
        check(prefix + '/declaring-prototype', () => typeof getter === 'function');
        check(prefix + '/concrete-placement', () => concrete === name ? !!descriptor :
          Object.getOwnPropertyDescriptor(realm[concrete].prototype, 'href') === undefined && !!descriptor);
        check(prefix + '/readonly-descriptor', () => descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
        check(prefix + '/function-realm', () => Object.getPrototypeOf(getter) === realm.Function.prototype);
        check(prefix + '/valid-native-receiver', () => invoke(getter, real) === value && value instanceof realm.SVGAnimatedString);
        check(prefix + '/stable-tear-off', () => invoke(getter, real) === invoke(getter, real));
        check(prefix + '/clone', () => invoke(getter, real.cloneNode(false)) instanceof realm.SVGAnimatedString);
        check(prefix + '/import', () => invoke(getter, doc.importNode(real, false)) instanceof realm.SVGAnimatedString);
        check(prefix + '/bypass-author-property', () => {
          const element = make(), original = element.href; let reads = 0;
          Object.defineProperty(element, 'href', {get() { reads++; throw 42; }});
          return invoke(getter, element) === original && reads === 0;
        });
        check(prefix + '/borrowed-foreign-getter', () => invoke(foreign, real) === value);
        check(prefix + '/borrowed-foreign-getter-cold', () => {
          const element = make(), first = invoke(foreign, element);
          return first === element.href && Object.getPrototypeOf(first) === realm.SVGAnimatedString.prototype;
        });
        check(prefix + '/foreign-error-realm', () => typeError(foreign, {}, other));
        const revoked = Proxy.revocable(real, {}); revoked.revoke();
        let traps = 0;
        const author = new Proxy(real, {
          get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }, has() { traps++; throw 42; },
        });
        const wrongTag = tag === 'image' ? 'use' : 'image';
        const wrong = doc.createElementNS(ns, wrongTag);
        Object.setPrototypeOf(wrong, proto);
        for (const [label, receiver] of [
          ['undefined', undefined], ['null', null], ['boolean', true], ['number', 42], ['symbol', Symbol('receiver')],
          ['plain', {}], ['interface-prototype', proto], ['forged-prototype', Object.create(proto)],
          ['inherit-real', Object.create(real)], ['author-proxy', author], ['revoked-proxy', revoked.proxy],
          ['wrong-uri-interface', doc.createElementNS(ns, wrongTag)], ['non-uri-svg', doc.createElementNS(ns, 'rect')],
          ['html-namespace', doc.createElementNS('http://www.w3.org/1999/xhtml', tag)], ['prototype-swapped-wrong-uri', wrong],
        ]) {
          check(prefix + '/receiver/' + label, () => typeError(getter, receiver, realm) && traps === 0);
        }
        check(prefix + '/retained-value-after-invalid-calls', () => real.href === value && value instanceof realm.SVGAnimatedString);
        check(prefix + '/default-empty', () => same(make().href, ''));
        check(prefix + '/attribute-forward-utf16', () => {
          const element = make(), original = element.href;
          element.setAttribute('href', '#read\ud800');
          return original === element.href && same(original, '#read\ud800');
        });
        check(prefix + '/xlink-fallback', () => {
          const element = make(), original = element.href;
          element.setAttributeNS(xlink, 'xlink:href', '#legacy\udc00');
          return original === element.href && same(original, '#legacy\udc00');
        });
        check(prefix + '/href-precedence', () => {
          const element = make(), original = element.href;
          element.setAttributeNS(xlink, 'xlink:href', '#legacy'); element.setAttribute('href', '#current');
          return same(original, '#current');
        });
        check(prefix + '/empty-href-precedence', () => {
          const element = make(), original = element.href;
          element.setAttributeNS(xlink, 'xlink:href', '#legacy'); element.setAttribute('href', '');
          return same(original, '');
        });
        check(prefix + '/remove-href-restores-xlink', () => {
          const element = make(), original = element.href;
          element.setAttributeNS(xlink, 'xlink:href', '#legacy'); element.setAttribute('href', '#current');
          element.removeAttribute('href'); return same(original, '#legacy');
        });
        check(prefix + '/remove-both-restores-empty', () => {
          const element = make(), original = element.href;
          element.setAttributeNS(xlink, 'xlink:href', '#legacy'); element.setAttribute('href', '#current');
          element.removeAttribute('href'); element.removeAttributeNS(xlink, 'href'); return same(original, '');
        });
        check(prefix + '/unqualified-xlink-spelling-ignored', () => {
          const element = make(); element.setAttribute('xlink:href', '#fake'); return same(element.href, '');
        });
        check(prefix + '/wrong-namespace-ignored', () => {
          const element = make(); element.setAttributeNS('urn:wrong', 'other:href', '#fake'); return same(element.href, '');
        });
        for (const state of ['absent','href-only','xlink-only','both']) {
          const setup = () => {
            const element = make();
            if (state === 'href-only' || state === 'both') element.setAttribute('href', '#current');
            if (state === 'xlink-only' || state === 'both') element.setAttributeNS(xlink, 'xlink:href', '#legacy');
            return element;
          };
          check(prefix + '/writeback/' + state, () => {
            const element = setup(), original = element.href;
            original.baseVal = '#written\ud800';
            return original === element.href && same(original, '#written\ud800') &&
              element.getAttributeNS(state === 'xlink-only' ? xlink : null, 'href') === '#written\ud800' &&
              (state !== 'xlink-only' || !element.hasAttribute('href')) &&
              (state !== 'both' || element.getAttributeNS(xlink, 'href') === '#legacy');
          });
          check(prefix + '/single-native-attribute-mutation/' + state, () => {
            const element = setup(), original = element.href;
            const observer = new realm.MutationObserver(() => {});
            observer.observe(element, {attributes: true, attributeOldValue: true});
            try {
              original.baseVal = '#written'; const records = observer.takeRecords();
              const oldValue = state === 'absent' ? null : state === 'xlink-only' ? '#legacy' : '#current';
              return records.length === 1 && records[0].target === element && records[0].attributeName === 'href' &&
                records[0].attributeNamespace === (state === 'xlink-only' ? xlink : null) && records[0].oldValue === oldValue;
            } finally { observer.disconnect(); }
          });
        }
        check(prefix + '/value-brand-before-conversion', () => {
          const element = make(), original = element.href;
          const setter = Object.getOwnPropertyDescriptor(realm.SVGAnimatedString.prototype, 'baseVal').set;
          let conversions = 0, reads = 0, error;
          const receiver = new Proxy(original, {get() { reads++; throw 42; }});
          try { setter.call(receiver, {toString() { conversions++; return '#forged'; }}); }
          catch (caught) { error = caught; }
          return error instanceof realm.TypeError && conversions === 0 && reads === 0 && !element.hasAttribute('href');
        });
        check(prefix + '/conversion-exception-no-mutation', () => {
          const element = make(), original = element.href, sentinel = {};
          element.setAttribute('href', '#current'); let error;
          try { original.baseVal = {toString() { throw sentinel; }}; } catch (caught) { error = caught; }
          return error === sentinel && element.getAttribute('href') === '#current' && same(original, '#current');
        });
        check(prefix + '/conversion-reselects-namespace', () => {
          const element = make(), original = element.href;
          element.setAttribute('href', '#current'); element.setAttributeNS(xlink, 'xlink:href', '#legacy');
          let conversions = 0;
          original.baseVal = {toString() { conversions++; element.removeAttribute('href'); return '#after'; }};
          return conversions === 1 && !element.hasAttribute('href') &&
            element.getAttributeNS(xlink, 'href') === '#after' && same(original, '#after');
        });
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
