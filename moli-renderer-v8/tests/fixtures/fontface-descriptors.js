async function fontFaceDescriptorsProbe(realm = globalThis) {
  const failures = [];
  let checks = 0;
  const check = (value, label) => { ++checks; if (!value) failures.push(label); };
  const throws = (callback, expected, label, name) => {
    try { callback(); check(false, label + ': accepted'); }
    catch (error) {
      check((error === expected || (typeof expected === 'function' && error instanceof expected)) &&
        (name === undefined || error.name === name), label + ': error');
    }
  };
  const defaults = {
    ascentOverride: 'normal', descentOverride: 'normal', display: 'auto',
    featureSettings: 'normal', lineGapOverride: 'normal', sizeAdjust: '100%',
    stretch: 'normal', style: 'normal', unicodeRange: 'U+0-10FFFF',
    variant: 'normal', variationSettings: 'normal', weight: 'normal',
  };
  const names = Object.keys(defaults);
  const make = options => new realm.FontFace('Descriptors', 'url(unused.ttf)', options);
  const initialStatus = make().status;
  for (const options of [undefined, null, {}, Object.fromEntries(names.map(n => [n, undefined]))]) {
    const face = make(options);
    for (const name of names) check(face[name] === defaults[name], 'default ' + name);
    check(face.status === initialStatus, 'default status');
  }
  const marker = new realm.Error('conversion marker');
  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(realm.FontFace.prototype, name);
    check(descriptor && descriptor.enumerable && descriptor.configurable &&
      descriptor.get.length === 0 && descriptor.set.length === 1, 'descriptor ' + name);
    if (!descriptor) continue;
    const face = make();
    const revoked = Proxy.revocable(face, {}); revoked.revoke();
    let conversions = 0, traps = 0;
    const value = {toString() { ++conversions; return defaults[name]; }};
    for (const receiver of [{}, Object.create(face), new Proxy(face, {get() { ++traps; }}), revoked.proxy]) {
      throws(() => descriptor.get.call(receiver), realm.TypeError, name + ' get receiver');
      throws(() => descriptor.set.call(receiver, value), realm.TypeError, name + ' set receiver');
    }
    check(conversions === 0 && traps === 0, name + ' receiver before conversion');
    throws(() => { face[name] = Symbol(); }, realm.TypeError, name + ' Symbol');
    check(face[name] === defaults[name], name + ' Symbol retains value');
    throws(() => { face[name] = {toString() { throw marker; }}; }, marker, name + ' conversion');
    check(face[name] === defaults[name], name + ' conversion retains value');
    throws(() => { face[name] = 'invalid'; }, realm.DOMException, name + ' invalid CSS', 'SyntaxError');
    check(face[name] === defaults[name] && face.status === initialStatus, name + ' invalid CSS retains state');
    let bad;
    try { bad = make({[name]: 'invalid'}); }
    catch (_) { check(false, name + ' constructor threw for CSS syntax'); continue; }
    check(bad.status === 'error', name + ' constructor error status');
    const loaded = bad.loaded;
    check(loaded === bad.loaded && bad.load() === loaded, name + ' stable rejection');
    const error = await loaded.catch(error => error);
    check(error instanceof realm.DOMException && error.name === 'SyntaxError', name + ' constructor rejection');
    // CSS Font Loading clears the descriptor that failed to parse.
    check(bad[name] === '', name + ' invalid constructor descriptor');
  }
  const valid = [
    ['style', 'ITALIC', 'italic'], ['style', 'oblique 20deg 30deg', 'oblique 20deg 30deg'],
    ['weight', '0700', '700'], ['weight', '100 900', '100 900'], ['weight', 'calc(500 + 200)', 'calc(700)'],
    ['stretch', 'condensed', 'condensed'], ['stretch', '75% 125%', '75% 125%'],
    ['variant', 'SMALL-CAPS', 'small-caps'],
    ['featureSettings', "'liga' off", '"liga" 0'], ['featureSettings', "'kern' on", '"kern"'],
    ['variationSettings', "'wght' 850", '"wght" 850'], ['display', 'SWAP', 'swap'],
    ['unicodeRange', 'u+0020-007f', 'U+20-7F'], ['unicodeRange', 'U+??', 'U+0-FF'],
    ['unicodeRange', 'U+10FFFF', 'U+10FFFF'], ['unicodeRange', 'U+41, U+42', 'U+41, U+42'],
    ['ascentOverride', '50.000%', '50%'], ['ascentOverride', 'calc(20% + 5%)', 'calc(25%)'],
    ['descentOverride', '200%', '200%'], ['lineGapOverride', '0%', '0%'],
    ['sizeAdjust', '100%', '100%'], ['sizeAdjust', 'calc(100% + 20%)', 'calc(120%)'],
  ];
  for (const [name, input, expected] of valid) {
    const face = make({[name]: input});
    check(face.status === initialStatus && face[name] === expected, name + ' constructor serialization: ' + input);
    face[name] = input;
    check(face[name] === expected, name + ' setter serialization: ' + input);
  }
  for (const [name, values] of Object.entries({
    unicodeRange: ['U+110000', 'U+41-40', 'U+1-FFFFFF', 'U+41 trailing'],
    weight: ['0', '1001', 'bolder', '400 !important'],
    ascentOverride: ['-1%', '0', '10px'], descentOverride: ['-5%'],
    lineGapOverride: ['1em'], sizeAdjust: ['-1%', 'normal'],
    style: ['italic; font-weight: 700', 'var(--style)', 'inherit'],
    variant: ['all-small-caps', 'common-ligatures small-caps'],
  })) for (const input of values) {
    const face = make();
    throws(() => { face[name] = input; }, realm.DOMException, name + ' grammar: ' + input, 'SyntaxError');
    check(face[name] === defaults[name], name + ' failed write: ' + input);
  }
  const order = [];
  const dictionary = new Proxy({}, {get(_, key) { order.push(key); return undefined; }});
  new realm.FontFace({toString() {order.push('family'); return 'A';}},
    {toString() {order.push('source'); return 'url(unused.ttf)';}}, dictionary);
  check(order.join() === ['family', 'source', ...names].join(), 'dictionary member order: ' + order);
  for (const [index, name] of names.entries()) {
    const reads = [];
    const dictionary = new Proxy({}, {get(_, key) {
      reads.push(key);
      if (key === name) throw marker;
      return undefined;
    }});
    throws(() => make(dictionary), marker, name + ' dictionary exception');
    check(reads.join() === names.slice(0, index + 1).join(), name + ' stops dictionary reads');
    throws(() => make({[name]: Symbol()}), realm.TypeError, name + ' dictionary Symbol');
  }
  for (const dictionary of [true, 42, '', 'text', Symbol()]) {
    throws(() => make(dictionary), realm.TypeError, 'dictionary type');
  }
  throws(() => make({ascentOverride: 'invalid', get weight() {throw marker;}}), marker,
    'convert entire dictionary before parsing CSS');
  class Derived extends realm.FontFace {}
  const derived = new Derived('Derived', 'url(unused.ttf)', {ascentOverride: '25%'});
  derived.unicodeRange = 'U+41';
  check(derived.ascentOverride === '25%' && derived.unicodeRange === 'U+41', 'subclass slots');
  return {checks, failures};
}
