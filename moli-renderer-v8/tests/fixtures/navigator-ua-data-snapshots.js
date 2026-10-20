(async () => {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({name, pass: !!run()}); }
    catch (error) { checks.push({name, pass: false, error: error.name}); }
  };
  const ua = navigator.userAgentData;
  const proto = NavigatorUAData.prototype;
  const low = ua.toJSON();
  check('interface has no own public members', () => Object.keys(ua).length === 0);
  for (const key of ['brands', 'mobile', 'platform']) {
    const descriptor = Object.getOwnPropertyDescriptor(proto, key);
    check(`${key} is a readonly prototype accessor`, () =>
      typeof descriptor?.get === 'function' && descriptor.set === undefined &&
      descriptor.enumerable && descriptor.configurable && !Object.hasOwn(ua, key));
    const previous = ua[key];
    const accepted = Reflect.set(ua, key, null);
    if (accepted) ua[key] = previous;
    check(`${key} rejects assignment`, () => !accepted);
    for (const [kind, receiver] of [
      ['ordinary', {}], ['forged', Object.create(proto)],
      ['inherited', Object.create(ua)], ['proxy', new Proxy(ua, {})]
    ]) {
      check(`${key} rejects ${kind} receiver`, () => {
        if (typeof descriptor?.get !== 'function') return false;
        try { descriptor.get.call(receiver); return false; }
        catch (error) { return error instanceof TypeError; }
      });
    }
  }
  const brands = ua.brands;
  check('interface brands array is frozen', () => Object.isFrozen(brands));
  check('interface brands array is a fresh frozen snapshot', () => brands !== ua.brands && Object.isFrozen(ua.brands));
  check('brand dictionary remains mutable', () => !Object.isFrozen(brands[0]));
  check('JSON has plain data members in IDL order', () =>
    Object.getPrototypeOf(low) === Object.prototype &&
    Object.keys(low).join() === 'brands,mobile,platform' &&
    Object.values(Object.getOwnPropertyDescriptors(low)).every(d =>
      d.writable && d.enumerable && d.configurable && !('get' in d)));
  check('JSON brand records have ordinary data properties', () =>
    Object.getPrototypeOf(low.brands[0]) === Object.prototype &&
    Object.keys(low.brands[0]).join() === 'brand,version' &&
    Object.values(Object.getOwnPropertyDescriptors(low.brands[0])).every(d =>
      d.writable && d.enumerable && d.configurable));
  check('JSON arrays and dictionaries are independent mutable copies', () => {
    const copy = ua.toJSON();
    copy.brands[0].brand = 'author change';
    copy.brands.length = 0;
    return !Object.isFrozen(low.brands) && brands !== low.brands &&
      ua.toJSON().brands[0].brand === low.brands[0].brand && brands.length > 0;
  });
  const firstBrand = brands[0].brand;
  brands[0].brand = 'changed dictionary';
  check('interface dictionary mutation does not change native identity', () =>
    ua.toJSON().brands[0].brand === firstBrand);
  brands[0].brand = firstBrand;
  let reads = 0;
  const hints = { [Symbol.iterator]() { reads++; return ['architecture', 'fullVersionList', 'architecture', 'unknown'][Symbol.iterator](); } };
  const high = await ua.getHighEntropyValues(hints);
  check('hints are iterated once', () => reads === 1);
  check('high entropy snapshot omits unrequested members', () =>
    Object.keys(high).join() === 'architecture,brands,fullVersionList,mobile,platform');
  check('high entropy arrays are fresh and mutable', () =>
    !Object.isFrozen(high.brands) && !Object.isFrozen(high.fullVersionList) &&
    high.brands !== low.brands && high.brands !== brands &&
    high.brands[0] !== brands[0]);
  let conversions = 0;
  try { await proto.getHighEntropyValues.call({}, { [Symbol.iterator]() { conversions++; throw 'converted'; } }); }
  catch (error) { check('receiver validation precedes hints conversion', () => error instanceof TypeError && conversions === 0); }
  const sentinel = {};
  try { await ua.getHighEntropyValues({[Symbol.iterator]() { throw sentinel; }}); }
  catch (error) { check('iterator exception retains identity', () => error === sentinel); }
  const expected = JSON.stringify(ua.toJSON());
  try {
    for (const key of ['brands', 'mobile', 'platform']) {
      Object.defineProperty(ua, key, {configurable: true, get() { throw new Error('author getter'); }});
    }
    check('JSON ignores author replacements of interface attributes', () => JSON.stringify(proto.toJSON.call(ua)) === expected);
    const result = await proto.getHighEntropyValues.call(ua, []);
    check('high entropy values ignore author replacements', () => JSON.stringify(result) === expected);
  } finally {
    for (const key of ['brands', 'mobile', 'platform']) delete ua[key];
  }
  let inheritedSets = 0;
  const ordinary = Object.prototype;
  const prior = Object.getOwnPropertyDescriptor(ordinary, 'brands');
  try {
    Object.defineProperty(ordinary, 'brands', {configurable: true, set() { inheritedSets++; }});
    const value = ua.toJSON();
    check('dictionary output bypasses inherited setters', () => inheritedSets === 0 && Object.hasOwn(value, 'brands'));
  } finally {
    if (prior) Object.defineProperty(ordinary, 'brands', prior);
    else delete ordinary.brands;
  }
  return {checks, passed: checks.filter(c => c.pass).length, total: checks.length};
})()
