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
  const lengths = names => names.map(name => [name, 'SVGAnimatedLength', name, 'length']);
  const graphics = [
    ['transform', 'SVGAnimatedTransformList', 'transform', 'transform'],
    ['requiredExtensions', 'SVGStringList', 'requiredExtensions', 'extensions'],
    ['systemLanguage', 'SVGStringList', 'systemLanguage', 'languages'],
  ];
  const geometry = [['pathLength', 'SVGAnimatedNumber', 'pathLength', 'number']];
  const definitions = [
    ['g', 'SVGGraphicsElement', graphics],
    ['path', 'SVGGeometryElement', geometry],
    ['svg', 'SVGSVGElement', lengths(['x','y','width','height'])],
    ['rect', 'SVGRectElement', lengths(['x','y','width','height','rx','ry'])],
    ['circle', 'SVGCircleElement', lengths(['cx','cy','r'])],
    ['ellipse', 'SVGEllipseElement', lengths(['cx','cy','rx','ry'])],
    ['line', 'SVGLineElement', lengths(['x1','y1','x2','y2'])],
    ['image', 'SVGImageElement', lengths(['x','y','width','height'])],
    ['use', 'SVGUseElement', lengths(['x','y','width','height'])],
    ['foreignObject', 'SVGForeignObjectElement', lengths(['x','y','width','height'])],
  ];
  const reflection = {
    length: {
      raw: '12',
      matches: value => value.baseVal.value === 12 && value.animVal.value === 12,
      write: value => { value.baseVal.value = 17; },
      written: value => value.baseVal.value === 17 && value.animVal.value === 17,
      serialized: '17',
    },
    number: {
      raw: '12.5',
      matches: value => value.baseVal === 12.5 && value.animVal === 12.5,
      write: value => { value.baseVal = 17.5; },
      written: value => value.baseVal === 17.5 && value.animVal === 17.5,
      serialized: '17.5',
    },
    transform: {
      raw: 'translate(2 3)',
      matches: value => value.baseVal.numberOfItems === 1 && value.animVal.numberOfItems === 1 &&
        value.baseVal.getItem(0).matrix.e === 2 && value.animVal.getItem(0).matrix.f === 3,
      write: value => { value.baseVal.getItem(0).setTranslate(5, 6); },
      written: value => value.baseVal.numberOfItems === 1 && value.animVal.numberOfItems === 1 &&
        value.baseVal.getItem(0).matrix.e === 5 && value.animVal.getItem(0).matrix.f === 6,
    },
    extensions: {
      raw: 'urn:a urn:b',
      matches: value => value.numberOfItems === 2 && value.getItem(0) === 'urn:a' && value.getItem(1) === 'urn:b',
      write: value => { value.initialize('urn:c'); },
      written: value => value.numberOfItems === 1 && value.getItem(0) === 'urn:c',
      serialized: 'urn:c',
    },
    languages: {
      raw: 'en,fr',
      matches: value => value.numberOfItems === 2 && value.getItem(0) === 'en' && value.getItem(1) === 'fr',
      write: value => { value.initialize('de'); },
      written: value => value.numberOfItems === 1 && value.getItem(0) === 'de',
      serialized: 'de',
    },
  };
  const serializedMatches = (element, property, attribute, kind, behavior) => {
    if (kind !== 'transform') return element.getAttribute(attribute) === behavior.serialized;
    // SVG2 serializes the matrix; Chromium currently retains translate syntax.
    // Check attribute round-trip semantics independently of that spelling.
    const copy = element.cloneNode(false), list = copy[property].baseVal;
    if (!element.getAttribute(attribute) || list.numberOfItems !== 1) return false;
    const matrix = list.getItem(0).matrix;
    return matrix.a === 1 && matrix.b === 0 && matrix.c === 0 && matrix.d === 1 && matrix.e === 5 && matrix.f === 6;
  };
  for (const [world, realm] of [['main', globalThis], ['child', child]]) {
    for (const [documentKind, doc] of [
      ['live', realm.document],
      ['windowless-html', realm.document.implementation.createHTMLDocument('')],
      ['windowless-xml', new realm.DOMParser().parseFromString('<root/>', 'application/xml')],
    ]) {
      for (const [tag, name, properties] of definitions) {
        const proto = realm[name].prototype;
        const make = () => doc.createElementNS(ns, tag);
        for (const [property, type, attribute, kind] of properties) {
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
          check(prefix + '/borrowed-foreign-getter-cold', () => {
            const element = make(), first = invoke(foreign, element);
            return first === element[property] && Object.getPrototypeOf(first) === realm[type].prototype;
          });
          check(prefix + '/foreign-error-realm', () => typeError(foreign, {}, other));
          const revoked = Proxy.revocable(real, {}); revoked.revoke();
          let traps = 0;
          const author = new Proxy(real, {
            get() { traps++; throw 42; },
            getPrototypeOf() { traps++; throw 42; },
            has() { traps++; throw 42; },
          });
          const wrongTag = tag === 'image' ? 'use' : tag === 'use' ? 'foreignObject' : 'marker';
          const wrong = doc.createElementNS(ns, wrongTag);
          Object.setPrototypeOf(wrong, proto);
          for (const [label, receiver] of [
            ['undefined', undefined], ['null', null], ['boolean', true], ['number', 42],
            ['symbol', Symbol('receiver')], ['plain', {}], ['interface-prototype', proto],
            ['forged-prototype', Object.create(proto)], ['inherit-real', Object.create(real)],
            ['author-proxy', author], ['revoked-proxy', revoked.proxy],
            ['wrong-svg', doc.createElementNS(ns, wrongTag)],
            ['html-namespace', doc.createElementNS('http://www.w3.org/1999/xhtml', tag)],
            ['prototype-swapped-wrong-svg', wrong],
          ]) {
            check(prefix + '/receiver/' + label, () => typeError(getter, receiver, realm) && traps === 0);
          }
          check(prefix + '/retained-value-after-invalid-calls', () => real[property] === value);
          const behavior = reflection[kind];
          check(prefix + '/attribute-forward-reflection', () => {
            const element = make(), original = element[property];
            element.setAttribute(attribute, behavior.raw);
            return original === element[property] && behavior.matches(original);
          });
          check(prefix + '/value-writeback', () => {
            const element = make(); element.setAttribute(attribute, behavior.raw);
            const original = element[property]; behavior.write(original);
            return original === element[property] && behavior.written(original) &&
              serializedMatches(element, property, attribute, kind, behavior);
          });
          check(prefix + '/single-native-attribute-mutation', () => {
            const element = make(); element.setAttribute(attribute, behavior.raw);
            const original = element[property];
            const observer = new realm.MutationObserver(() => {});
            observer.observe(element, {attributes: true, attributeOldValue: true});
            try {
              behavior.write(original);
              const records = observer.takeRecords();
              return records.length === 1 && records[0].target === element && records[0].attributeName === attribute &&
                records[0].attributeNamespace === null && records[0].oldValue === behavior.raw;
            } finally { observer.disconnect(); }
          });
        }
      }
      for (const [name, tags, properties] of [
        ['SVGGraphicsElement', ['svg','g','defs','symbol','use','switch','path','rect','circle','ellipse','line',
          'polyline','polygon','text','tspan','textPath','image','foreignObject','a'], graphics],
        ['SVGGeometryElement', ['path','rect','circle','ellipse','line','polyline','polygon'], geometry],
      ]) {
        for (const tag of tags) {
          for (const [property, type] of properties) {
            const element = doc.createElementNS(ns, tag);
            const getter = Object.getOwnPropertyDescriptor(realm[name].prototype, property).get;
            const prefix = `${world}/${documentKind}/inheritance/${name}/${tag}/${property}`;
            check(prefix + '/native-subclass', () => invoke(getter, element) === element[property]);
            check(prefix + '/producer-realm', () => Object.getPrototypeOf(invoke(getter, element)) === realm[type].prototype);
            check(prefix + '/prototype-change-preserves-brand', () => {
              const original = invoke(getter, element);
              Object.setPrototypeOf(element, realm.Object.prototype);
              return invoke(getter, element) === original;
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
