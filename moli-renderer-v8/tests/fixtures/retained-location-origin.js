async ({parentFirst = false} = {}) => {
  const result = {checks: 0, failures: [], observations: []};
  const check = (name, action, expected) => {
    let actual;
    try { actual = action(); } catch (error) { actual = error.name; }
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const load = async path => {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = path;
    document.body.append(frame);
    await loaded;
    return frame;
  };
  const frame = await load('/child.html#seed');
  const peerFrame = await load('/peer.html');
  const child = frame.contentWindow;
  const childDocument = child.document;
  const parentLocationIdentity = child.Function('expected', 'return parent.location === expected').bind(null, location);
  const parentLocationRead = child.Function('held', 'try { return held.host; } catch (error) { return error.name; }').bind(null, location);
  const peer = peerFrame.contentWindow;
  const peerDocument = peer.document;
  const held = child.location;
  const href = held.href;
  const proto = Object.getPrototypeOf(held);
  const descriptors = Object.getOwnPropertyDescriptors(held);
  const peerRead = peer.Function('target', 'return target.replace');
  const peerFunctionPrototype = peer.Function.prototype;
  const parentExceptionPrototype = DOMException.prototype;
  const childExceptionPrototype = child.DOMException.prototype;
  const childTypeErrorPrototype = child.TypeError.prototype;
  const ownReplace = held.replace;
  const childObjectPrototype = child.Object.prototype;
  childObjectPrototype.get = () => 'polluted';
  check('handler-does-not-inherit-get', () => held.href, href);
  delete childObjectPrototype.get;
  childObjectPrototype.getPrototypeOf = () => null;
  check('handler-does-not-inherit-get-prototype', () => Object.getPrototypeOf(held) === proto, true);
  delete childObjectPrototype.getPrototypeOf;
  const symbol = Symbol('retained-location');
  held.marker = 42;
  held[0] = 'index';
  held[4294967295] = 'non-index';
  held[symbol] = 'symbol';
  held.then = 'author then';
  Object.defineProperty(held, 'locked', {value: 123});
  let expandoReads = 0;
  Object.defineProperty(held, 'accessor', {
    get() { expandoReads++; return 1; }, configurable: true
  });
  check('initial-identity', () => held === child.location, true);
  check('initial-marker', () => held.marker, 42);
  if (parentFirst) {
    document.domain = 'example.test';
    peerDocument.domain = 'example.test';
  } else {
    childDocument.domain = 'example.test';
  }
  check('window-access-denied', () => child.status, 'SecurityError');
  check('parent-location-identity', () => parentLocationIdentity(), true);
  check('retained-parent-location-security', () => parentLocationRead(), 'SecurityError');
  check('location-identity-survives-origin-change', () => child.location === held, true);
  const crossWindowLocationGetter = Object.getOwnPropertyDescriptor(child, 'location').get;
  check('cross-window-location-getter', () => crossWindowLocationGetter.call(child) === held, true);
  check('cross-window-location-rejects-document', () => crossWindowLocationGetter.call(document), 'TypeError');
  for (const key of ['marker', 'locked', 'accessor', 'missing', 0, 4294967295, symbol,
    'href', 'host', 'hostname', 'port', 'protocol', 'hash', 'search', 'pathname',
    'origin', 'ancestorOrigins', 'assign', 'reload', 'toString', 'valueOf', Symbol.toPrimitive]) {
    check(`read:${String(key)}`, () => held[key], 'SecurityError');
  }
  check('blocked-expando-getter-not-called', () => expandoReads, 0);
  for (const key of ['marker', 0, symbol, 'then', 'replace']) {
    check(`set:${String(key)}`, () => Reflect.set(held, key, 7), 'SecurityError');
    check(`define:${String(key)}`, () => Reflect.defineProperty(held, key, {value: 7}), 'SecurityError');
    check(`delete:${String(key)}`, () => Reflect.deleteProperty(held, key), 'SecurityError');
  }
  check('has-expando', () => 'marker' in held, 'SecurityError');
  check('has-href', () => 'href' in held, true);
  check('has-fallback', () => 'then' in held, true);
  check('descriptor-expando', () => Object.getOwnPropertyDescriptor(held, 'marker'), 'SecurityError');
  for (const key of ['locked', 'missing', 0, symbol]) {
    check(`descriptor:${String(key)}`, () => Object.getOwnPropertyDescriptor(held, key), 'SecurityError');
  }
  check('cross-origin-prototype', () => Object.getPrototypeOf(held), null);
  check('set-null-prototype', () => Reflect.setPrototypeOf(held, null), true);
  check('set-original-prototype', () => Reflect.setPrototypeOf(held, proto), false);
  check('extensible', () => Reflect.isExtensible(held), true);
  check('prevent-extensions', () => Reflect.preventExtensions(held), false);
  check('own-keys', () => Reflect.ownKeys(held).map(String).join('|'),
    'href|replace|then|Symbol(Symbol.toStringTag)|Symbol(Symbol.hasInstance)|Symbol(Symbol.isConcatSpreadable)');
  check('enumerable-keys', () => Object.keys(held).join('|'), '');
  for (const key of ['then', Symbol.toStringTag, Symbol.hasInstance, Symbol.isConcatSpreadable]) {
    check(`fallback:${String(key)}`, () => held[key] === undefined, true);
  }
  check('href-descriptor', () => {
    const d = Object.getOwnPropertyDescriptor(held, 'href');
    return [typeof d.get, typeof d.set, d.enumerable, d.configurable,
      Object.getPrototypeOf(d.set) === Function.prototype, d.set.name, d.set.length].join('|');
  }, 'undefined|function|false|true|true|set href|1');
  check('replace-descriptor', () => {
    const d = Object.getOwnPropertyDescriptor(held, 'replace');
    return [typeof d.value, d.enumerable, d.configurable, d.writable,
      Object.getPrototypeOf(d.value) === Function.prototype, d.value.name, d.value.length].join('|');
  }, 'function|false|true|false|true|replace|1');
  check('cross-replace-cached', () => held.replace === held.replace, true);
  check('cross-replace-new-function', () => held.replace !== ownReplace, true);
  check('cross-replace-descriptor-cached', () => held.replace === Object.getOwnPropertyDescriptor(held, 'replace').value, true);
  check('cross-setter-cached', () => Object.getOwnPropertyDescriptor(held, 'href').set === Object.getOwnPropertyDescriptor(held, 'href').set, true);
  check('peer-cross-replace-realm', () => Object.getPrototypeOf(peerRead(held)) === peerFunctionPrototype, true);
  check('peer-cross-replace-cached', () => peerRead(held) === peerRead(held), true);
  check('distinct-cross-caller-functions', () => peerRead(held) !== held.replace, true);
  for (const [key, d] of Object.entries(descriptors)) {
    if (d.get) check(`cached-getter:${key}`, () => d.get.call(held), 'SecurityError');
  }
  check('cached-stringifier', () => descriptors.toString.value.call(held), 'SecurityError');
  check('cached-hash-setter', () => { descriptors.hash.set.call(held, '#seed'); return 'accepted'; }, 'SecurityError');
  for (const [label, action, expectedPrototype] of [
    ['property-security-realm', () => held.host, parentExceptionPrototype],
    ['cached-getter-security-realm', () => descriptors.host.get.call(held), childExceptionPrototype]
  ]) {
    check(label, () => {
      try { action(); return false; }
      catch (error) { return error.name === 'SecurityError' && Object.getPrototypeOf(error) === expectedPrototype; }
    }, true);
  }
  let conversions = 0;
  const sentinel = {name: 'ConversionSentinel'};
  const poison = {toString() { conversions++; throw sentinel; }};
  for (const [label, action, expectedConversions] of [
    ['direct-hash', () => { held.hash = poison; }, 0],
    ['direct-href', () => { held.href = poison; }, 1],
    ['direct-replace', () => held.replace(poison), 1],
    ['cached-assign', () => descriptors.assign.value.call(held, poison), 1],
    ['cached-hash', () => descriptors.hash.set.call(held, poison), 1]
  ]) {
    const before = conversions;
    check(`conversion:${label}`, () => { action(); return 'accepted'; }, expectedConversions ? 'ConversionSentinel' : 'SecurityError');
    check(`conversion-count:${label}`, () => conversions - before, expectedConversions);
  }
  for (const receiver of [{}, Object.create(held), new Proxy(held, {}), 1, null]) {
    const before = conversions;
    check('reflect-cross-setter-brand', () => Reflect.set(held, 'href', poison, receiver), 'TypeError');
    check('reflect-cross-setter-conversion', () => conversions, before);
  }
  const revoked = Proxy.revocable(held, {});
  revoked.revoke();
  for (const receiver of [{}, Object.create(held), new Proxy(held, {}), revoked.proxy]) {
    const before = conversions;
    check('cached-getter-brand', () => {
      try { descriptors.href.get.call(receiver); return false; }
      catch (error) { return Object.getPrototypeOf(error) === childTypeErrorPrototype; }
    }, true);
    check('cached-setter-brand', () => descriptors.href.set.call(receiver, poison), 'TypeError');
    check('cached-method-brand', () => descriptors.replace.value.call(receiver, poison), 'TypeError');
    check('brand-before-conversion', () => conversions, before);
  }
  const waitForHref = async target => {
    for (let i = 0; i < 100; i++) {
      if (childDocument.URL === target) return true;
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    return false;
  };
  for (const [label, navigate] of [
    ['href', url => { held.href = url; }],
    ['replace', url => held.replace(url)],
    ['cached-href', url => descriptors.href.set.call(held, url)],
    ['cached-replace', url => descriptors.replace.value.call(held, url)]
  ]) {
    const target = new URL(href);
    target.hash = label;
    check(`allowed-navigation:${label}`, () => { navigate(target.href); return true; }, true);
    const reached = await waitForHref(target.href);
    check(`navigation-completed:${label}`, () => reached, true);
  }
  check('cached-assign', () => { descriptors.assign.value.call(held, href); return 'accepted'; }, 'SecurityError');
  check('cached-reload', () => { descriptors.reload.value.call(held); return 'accepted'; }, 'SecurityError');
  check('cached-method-security-realm', () => {
    try { descriptors.reload.value.call(held); return false; }
    catch (error) { return error.name === 'SecurityError' && Object.getPrototypeOf(error) === childExceptionPrototype; }
  }, true);
  if (parentFirst) childDocument.domain = 'example.test';
  else document.domain = 'example.test';
  check('same-origin-restored-identity', () => child.location === held, true);
  check('same-origin-restored-marker', () => held.marker, 42);
  check('same-origin-restored-locked', () => held.locked, 123);
  check('same-origin-restored-prototype', () => Object.getPrototypeOf(held) === proto, true);
  check('same-origin-restored-method', () => held.replace === ownReplace, true);
  check('same-origin-restored-getter', () => descriptors.href.get.call(held) === childDocument.URL, true);
  frame.remove();
  peerFrame.remove();
  return result;
}
