(() => {
  const failures = [];
  const check = (name, fn) => {
    try { if (!fn()) failures.push(name); }
    catch (error) { failures.push(name + ': ' + error); }
  };
  const foreign = document.querySelector('iframe').contentWindow;
  const definitions = [
    ['a', 'SVGAElement'], ['image', 'SVGImageElement'], ['use', 'SVGUseElement'],
    ['textPath', 'SVGTextPathElement'], ['pattern', 'SVGPatternElement'],
    ['script', 'SVGScriptElement'], ['linearGradient', 'SVGGradientElement'],
    ['filter', 'SVGFilterElement'], ['mpath', 'SVGMPathElement'],
  ];
  const docs = [document, foreign.document,
    document.implementation.createHTMLDocument(''),
    foreign.document.implementation.createHTMLDocument('')];
  for (const [tag, name] of definitions) {
    for (const callee of [window, foreign]) {
      const descriptor = Object.getOwnPropertyDescriptor(callee[name].prototype, 'href');
      check(name + '/descriptor', () => typeof descriptor.get === 'function' &&
        descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
      const getter = descriptor.get;
      for (const doc of docs) {
        const element = doc.createElementNS('http://www.w3.org/2000/svg', tag);
        const producer = doc === foreign.document || doc === docs[3] ? foreign : window;
        check(name + '/producer-realm', () =>
          Object.getPrototypeOf(getter.call(element)) === producer.SVGAnimatedString.prototype);
        check(name + '/same-object', () => getter.call(element) === element.href);
        check(name + '/native-writeback', () => {
          getter.call(element).baseVal = 'first';
          element.setAttribute('href', 'second');
          return element.getAttribute('href') === 'second' && getter.call(element).baseVal === 'second';
        });
        let traps = 0;
        const proxy = new Proxy(element, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
        const revoked = Proxy.revocable(element, {}); revoked.revoke();
        for (const receiver of [{}, doc.body, Object.create(element), Object.create(callee[name].prototype), proxy, revoked.proxy]) {
          check(name + '/invalid-receiver', () => {
            try {getter.call(receiver);} catch (error) {
              return Object.getPrototypeOf(error) === callee.TypeError.prototype && traps === 0;
            }
            return false;
          });
        }
        const wrong = doc.createElementNS('http://www.w3.org/2000/svg', tag === 'a' ? 'image' : 'a');
        check(name + '/wrong-svg-interface', () => {
          try {getter.call(wrong);} catch (error) {
            return Object.getPrototypeOf(error) === callee.TypeError.prototype;
          }
          return false;
        });
      }
      if (globalThis.__svgNativeUris) {
        const {target, proxy} = __svgNativeUris[tag];
        check(name + '/registered-proxy', () => {
          const value = getter.call(proxy);
          value.baseVal = 'native';
          return value === target.href && target.getAttribute('href') === 'native' &&
            Object.getPrototypeOf(value) === foreign.SVGAnimatedString.prototype;
        });
      }
    }
  }
  globalThis.__svgUriFailures = failures;
  return true;
})()
