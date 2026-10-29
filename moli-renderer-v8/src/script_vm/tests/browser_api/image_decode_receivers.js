globalThis.__imageDecodeReceiverResults = {complete: false, checks: []};
globalThis.__imageDecodeReceiverDone = (async () => {
  const results = __imageDecodeReceiverResults;
  const other = __imageDecodeReceiverOtherWindow;
  const realms = [window, other];
  const make = (doc, name) => name === 'HTMLImageElement'
    ? doc.createElement('img')
    : doc.createElementNS('http://www.w3.org/2000/svg', 'image');
  const check = async (name, run) => {
    try {
      results.checks.push({name, ...await run()});
    } catch (error) {
      results.checks.push({name, passed: false, error: String(error)});
    }
  };
  const surfaces = realms.flatMap((realm, index) => ['HTMLImageElement', 'SVGImageElement'].map(name => ({
    realm, index, name, constructor: realm[name],
    promisePrototype: realm.Promise.prototype,
    typeErrorPrototype: realm.TypeError.prototype,
    exceptionPrototype: realm.DOMException.prototype,
    descriptor: Object.getOwnPropertyDescriptor(realm[name].prototype, 'decode')
  })));
  const reject = async (surface, receiver, errorName, resultRealm = surface) => {
    const fn = surface.descriptor?.value;
    if (typeof fn !== 'function') throw Error('decode method missing');
    let promise;
    try {
      promise = Reflect.apply(fn, receiver, []);
    } catch (error) {
      return {passed: false, synchronous: true, errorName: error.name};
    }
    let error;
    try { await promise; } catch (caught) { error = caught; }
    const promiseRealm = Object.getPrototypeOf(promise) === resultRealm.promisePrototype;
    const errorRealm = !!error && Object.getPrototypeOf(error) ===
      (errorName === 'TypeError' ? resultRealm.typeErrorPrototype : resultRealm.exceptionPrototype);
    return {passed: promiseRealm && errorRealm && error.name === errorName,
            promiseRealm, errorRealm, errorName: error?.name};
  };
  for (const surface of surfaces) {
    const {realm, index, name, constructor, descriptor} = surface;
    const prefix = `${index}:${name}:`;
    const real = make(realm.document, name);
    await check(prefix + 'descriptor', () => {
      const fn = descriptor?.value;
      if (typeof fn !== 'function') throw Error('decode method missing');
      let constructError;
      try { Reflect.construct(fn, []); } catch (error) { constructError = error; }
      return {passed: descriptor.writable && descriptor.enumerable && descriptor.configurable &&
        fn.name === 'decode' && fn.length === 0 && !Object.hasOwn(fn, 'prototype') &&
        Function.prototype.toString.call(fn).includes('[native code]') && constructError?.name === 'TypeError'};
    });
    const altered = make(realm.document, name);
    Object.setPrototypeOf(altered, null);
    const inactive = realm.document.implementation.createHTMLDocument('');
    for (const [label, receiver] of [
      ['local', real], ['foreign', make(realms[1-index].document, name)],
      ['changed-prototype', altered], ['inactive-document', make(inactive, name)]
    ]) await check(prefix + label, () => reject(surface, receiver, 'EncodingError',
      label === 'foreign' ? surfaces.find(other => other.index === 1-index && other.name === name) : surface));
    let traps = 0;
    const trap = () => { traps++; throw Error('author Proxy trap'); };
    const author = new Proxy(real, {get: trap, getPrototypeOf: trap, has: trap});
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    const wrong = make(realm.document, name === 'HTMLImageElement' ? 'SVGImageElement' : 'HTMLImageElement');
    for (const [label, receiver] of [
      ['null', null], ['undefined', undefined], ['number', 1], ['boolean', true], ['string', 'image'],
      ['object', {}], ['prototype', constructor.prototype],
      ['forged-prototype', Object.create(constructor.prototype)], ['inherited-object', Object.create(real)],
      ['author-proxy', author], ['revoked-proxy', revoked.proxy], ['wrong-interface', wrong],
      ['text-node', realm.document.createTextNode('image')]
    ]) await check(prefix + label, async () => {
      const outcome = await reject(surface, receiver, 'TypeError');
      return {...outcome, traps, passed: outcome.passed && traps === 0};
    });
    await check(prefix + 'author-constructors', async () => {
      const keys = ['Promise', 'TypeError', 'DOMException', name];
      const saved = keys.map(key => [key, Object.getOwnPropertyDescriptor(realm, key)]);
      let calls = 0;
      const replacement = function AuthorConstructor() { calls++; throw Error('author constructor'); };
      try {
        for (const key of keys) realm[key] = replacement;
        const replaced = keys.every(key => realm[key] === replacement);
        const invalid = await reject(surface, {}, 'TypeError');
        const valid = await reject(surface, real, 'EncodingError');
        return {passed: replaced && invalid.passed && valid.passed && calls === 0, replaced, calls, invalid, valid};
      } finally {
        for (const [key, descriptor] of saved) Object.defineProperty(realm, key, descriptor);
      }
    });
  }
  return results;
})().then(results => {
  results.complete = true;
  return results;
}, error => {
  __imageDecodeReceiverResults.complete = true;
  __imageDecodeReceiverResults.error = String(error);
  return __imageDecodeReceiverResults;
});
