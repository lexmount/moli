(() => {
  const checks = [], traces = [];
  const hasOwn = Function.call.bind(Object.prototype.hasOwnProperty);
  const is = Object.is;
  const keys = ['fullRange', 'matrix', 'primaries', 'transfer'];
  const values = {
    primaries: ['bt709', 'bt470bg', 'smpte170m', 'bt2020', 'smpte432'],
    transfer: ['bt709', 'smpte170m', 'iec61966-2-1', 'linear', 'pq', 'hlg'],
    matrix: ['rgb', 'bt709', 'bt470bg', 'smpte170m', 'bt2020-ncl'],
  };
  const equal = (actual, expected) => {
    if (!is(actual, expected)) throw Error(`expected ${String(expected)}, got ${String(actual)}`);
  };
  const throws = (realm, name, run) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    if (!error || error.name !== name || !(error instanceof realm[name === 'TypeError' ? 'TypeError' : 'DOMException']))
      throw Error(`expected ${name} in the callee realm, got ${error}`);
    if (realm !== globalThis && name === 'TypeError' && error instanceof TypeError) throw Error('wrong TypeError realm');
    return error;
  };
  const realms = [globalThis];
  if (typeof document !== 'undefined') realms.push(document.getElementById('child').contentWindow);
  for (const [index, realm] of realms.entries()) {
    const C = realm.VideoColorSpace, prototype = C.prototype;
    const get = name => Object.getOwnPropertyDescriptor(prototype, name).get;
    const json = prototype.toJSON;
    const test = (name, run, category = 'shared') => {
      try {
        if (run() === false) throw Error('assertion returned false');
        checks.push({realm: index, name, category, passed: true});
      } catch (error) {
        checks.push({realm: index, name, category, passed: false, error: String(error), stack: error.stack});
      }
    };
    const fields = (object, expected) => {
      const result = json.call(object);
      equal(Object.keys(result).join(','), keys.join(','));
      for (const name of keys) {
        const value = hasOwn(expected, name) ? expected[name] ?? null : null;
        equal(get(name).call(object), value);
        equal(result[name], value);
        const d = Object.getOwnPropertyDescriptor(result, name);
        equal(d.enumerable, true); equal(d.writable, true); equal(d.configurable, true);
        equal(d.get, undefined); equal(d.set, undefined);
      }
    };
    test('interface and prototype metadata', () => {
      equal(C.name, 'VideoColorSpace'); equal(C.length, 0);
      equal(Object.getPrototypeOf(C), realm.Function.prototype);
      equal(Object.getPrototypeOf(prototype), realm.Object.prototype);
      equal(prototype.constructor, C);
      const d = Object.getOwnPropertyDescriptor(realm, 'VideoColorSpace');
      equal(d.enumerable, false); equal(d.writable, true); equal(d.configurable, true);
      const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
      equal(tag.value, 'VideoColorSpace'); equal(tag.enumerable, false); equal(tag.writable, false); equal(tag.configurable, true);
      const m = Object.getOwnPropertyDescriptor(prototype, 'toJSON');
      equal(m.enumerable, true); equal(m.writable, true); equal(m.configurable, true);
      equal(json.name, 'toJSON'); equal(json.length, 0);
      equal(Object.getPrototypeOf(json), realm.Function.prototype);
    });
    for (const name of keys) test(`readonly descriptor ${name}`, () => {
      const d = Object.getOwnPropertyDescriptor(prototype, name);
      equal(d.enumerable, true); equal(d.configurable, true); equal(d.set, undefined);
      equal(d.get.name, `get ${name}`); equal(d.get.length, 0);
      equal(Object.getPrototypeOf(d.get), realm.Function.prototype);
      const c = new C();
      equal(Reflect.set(c, name, 'author'), false); equal(get(name).call(c), null);
    });
    for (const [label, args] of [['omitted', []], ['undefined', [undefined]], ['null', [null]], ['empty', [{}]], ['null prototype', [Object.create(null)]]])
      test(`empty dictionary ${label}`, () => {
        const c = Reflect.construct(C, args); fields(c, {});
        equal(Object.getPrototypeOf(c), prototype); equal(Object.keys(c).length, 0);
        equal(Object.prototype.toString.call(c), '[object VideoColorSpace]');
      });
    for (const [label, value] of [['number', 1], ['boolean', true], ['string', 'bt709'], ['bigint', 1n], ['symbol', Symbol()]])
      test(`primitive dictionary rejects ${label}`, () => throws(realm, 'TypeError', () => new C(value)));
    test('requires new before reading dictionary', () => {
      let reads = 0;
      throws(realm, 'TypeError', () => C(new Proxy({}, {get() {reads++;}})));
      equal(reads, 0);
    });
    test('array and callable dictionaries are objects', () => {
      const array = []; array.matrix = 'rgb'; fields(new C(array), {matrix: 'rgb'});
      function init() {} init.primaries = 'bt2020'; fields(new C(init), {primaries: 'bt2020'});
    });
    test('dictionary getter and enum conversion order', () => {
      const log = [], init = {};
      for (const name of keys) Object.defineProperty(init, name, {get() {
        log.push(name);
        if (name === 'fullRange') return false;
        return {toString() {log.push(`${name}.string`); return values[name][0];}};
      }});
      fields(new C(init), {fullRange: false, matrix: 'rgb', primaries: 'bt709', transfer: 'bt709'});
      equal(log.join(','), 'fullRange,matrix,matrix.string,primaries,primaries.string,transfer,transfer.string');
      traces.push({realm: index, log});
    });
    test('inherited dictionary members are read', () => {
      fields(new C(Object.create({fullRange: false, matrix: 'bt470bg', primaries: 'smpte432', transfer: 'hlg'})),
        {fullRange: false, matrix: 'bt470bg', primaries: 'smpte432', transfer: 'hlg'});
    });
    test('nullish dictionary ignores polluted Object.prototype', () => {
      const descriptors = keys.map(name => Object.getOwnPropertyDescriptor(realm.Object.prototype, name));
      let reads = 0;
      try {
        for (const name of keys) Object.defineProperty(realm.Object.prototype, name, {configurable: true, get() {reads++; return name === 'fullRange' ? true : values[name][0];}});
        for (const args of [[], [null], [undefined]]) fields(Reflect.construct(C, args), {});
        equal(reads, 0);
        const real = new realm.Object();
        fields(new C(real), {fullRange: true, matrix: 'rgb', primaries: 'bt709', transfer: 'bt709'});
        equal(reads, 4);
      } finally {
        for (let i = 0; i < keys.length; i++) {
          if (descriptors[i]) Object.defineProperty(realm.Object.prototype, keys[i], descriptors[i]);
          else delete realm.Object.prototype[keys[i]];
        }
      }
    });
    test('null and undefined members use nullable defaults', () => {
      for (const value of [null, undefined]) fields(new C(Object.fromEntries(keys.map(name => [name, value]))), {});
    });
    for (const [name, tokens] of Object.entries(values)) {
      for (const token of tokens) test(`accepted ${name} ${token}`, () => fields(new C({[name]: token}), {[name]: token}));
      for (const [label, value] of [['unknown', 'other'], ['uppercase', tokens[0].toUpperCase()], ['space', ` ${tokens[0]}`], ['empty', ''], ['null text', 'null'], ['undefined text', 'undefined'], ['number', 1], ['boolean', true], ['bigint', 1n], ['symbol', Symbol()], ['unpaired surrogate', '\ud800']])
        test(`invalid ${name} ${label}`, () => throws(realm, 'TypeError', () => new C({[name]: value})));
      test(`enum string conversion hint ${name}`, () => {
        const log = [];
        fields(new C({[name]: {[Symbol.toPrimitive](hint) {log.push(hint); return tokens[0];}}}), {[name]: tokens[0]});
        equal(log.join(','), 'string');
      });
      test(`enum conversion preserves author exception ${name}`, () => {
        const marker = {};
        let caught;
        try {new C({[name]: {toString() {throw marker;}}});} catch (error) {caught = error;}
        equal(caught, marker);
      });
    }
    for (const [label, value, expected] of [['false', false, false], ['true', true, true], ['zero', 0, false], ['negative zero', -0, false], ['one', 1, true], ['nan', NaN, false], ['empty', '', false], ['text', 'false', true], ['zero bigint', 0n, false], ['bigint', 1n, true], ['symbol', Symbol(), true], ['wrapped false', new Boolean(false), true], ['object', {}, true]])
      test(`nullable boolean ToBoolean ${label}`, () => fields(new C({fullRange: value}), {fullRange: expected}));
    test('boolean conversion never invokes author primitive hooks', () => {
      let conversions = 0;
      fields(new C({fullRange: {[Symbol.toPrimitive]() {conversions++; throw Error('unexpected conversion');}}}), {fullRange: true});
      equal(conversions, 0);
    });
    for (const stop of keys) test(`getter exception stops at ${stop}`, () => {
      const marker = {}, log = [], init = {};
      for (const name of keys) Object.defineProperty(init, name, {get() {log.push(name); if (name === stop) throw marker; return undefined;}});
      let caught;
      try {new C(init);} catch (error) {caught = error;}
      equal(caught, marker); equal(log.join(','), keys.slice(0, keys.indexOf(stop) + 1).join(','));
    });
    test('invalid matrix stops before later members', () => {
      const log = [];
      throws(realm, 'TypeError', () => new C(new Proxy({}, {get(t, name) {log.push(name); return name === 'matrix' ? 'invalid' : undefined;}})));
      equal(log.join(','), 'fullRange,matrix');
    });
    test('constructor snapshots converted members', () => {
      const init = {primaries: 'bt709', transfer: 'pq', matrix: 'rgb', fullRange: true};
      const c = new C(init); init.primaries = 'smpte432'; init.transfer = 'hlg'; init.matrix = 'bt2020-ncl'; init.fullRange = false;
      fields(c, {primaries: 'bt709', transfer: 'pq', matrix: 'rgb', fullRange: true});
    });
    test('toJSON uses native getters and independent output', () => {
      const expected = {primaries: 'bt2020', transfer: 'hlg', matrix: 'bt2020-ncl', fullRange: false};
      const c = new C(expected);
      for (const name of keys) Object.defineProperty(c, name, {get() {throw Error('public getter');}});
      c.authorExtra = 'ignored'; c[Symbol()] = 'ignored';
      const first = json.call(c), second = json.call(c);
      equal(first === second, false); first.primaries = 'changed'; delete first.matrix;
      fields(c, expected); equal(Object.keys(second).join(','), keys.join(','));
      for (const name of keys) equal(second[name], expected[name]);
    });
    test('toJSON data properties bypass prototype setters', () => {
      const descriptors = keys.map(name => Object.getOwnPropertyDescriptor(realm.Object.prototype, name));
      const c = new C(); let calls = 0;
      try {
        for (const name of keys) Object.defineProperty(realm.Object.prototype, name, {configurable: true, set() {calls++; throw Error('inherited setter');}});
        fields(c, {}); equal(calls, 0);
      } finally {
        for (let i = 0; i < keys.length; i++) {
          if (descriptors[i]) Object.defineProperty(realm.Object.prototype, keys[i], descriptors[i]);
          else delete realm.Object.prototype[keys[i]];
        }
      }
    });
    test('toJSON ignores replaced interface getters', () => {
      const c = new C({matrix: 'rgb'}), descriptors = keys.map(name => Object.getOwnPropertyDescriptor(prototype, name));
      try {
        for (const name of keys) Object.defineProperty(prototype, name, {configurable: true, get() {throw Error('prototype getter');}});
        const result = json.call(c); equal(result.matrix, 'rgb'); equal(result.primaries, null);
      } finally {for (let i = 0; i < keys.length; i++) Object.defineProperty(prototype, keys[i], descriptors[i]);}
    });
    test('brand survives public prototype replacement', () => {
      const c = new C({primaries: 'smpte432'}); Object.setPrototypeOf(c, null); fields(c, {primaries: 'smpte432'});
    });
    const real = new C(), revoked = Proxy.revocable(real, {}); revoked.revoke();
    const receivers = [['ordinary', {}], ['prototype', prototype], ['forged prototype', Object.create(prototype)], ['inherits real', Object.create(real)], ['author proxy', new Proxy(real, {})], ['revoked proxy', revoked.proxy], ['null', null], ['undefined', undefined]];
    for (const [label, receiver] of receivers) {
      for (const name of keys) test(`getter receiver ${name} ${label}`, () => throws(realm, 'TypeError', () => get(name).call(receiver)));
      test(`toJSON receiver ${label}`, () => throws(realm, 'TypeError', () => json.call(receiver)));
    }
    test('receiver brands do not invoke author proxy traps', () => {
      let traps = 0;
      const proxy = new Proxy(real, {get() {traps++; throw Error('get trap');}, getPrototypeOf() {traps++; throw Error('prototype trap');}});
      throws(realm, 'TypeError', () => json.call(proxy));
      for (const name of keys) throws(realm, 'TypeError', () => get(name).call(proxy));
      equal(traps, 0);
    });
    test('unused arguments are ignored for constructor and method', () => {
      let reads = 0; const excess = new Proxy({}, {get() {reads++; throw Error('excess');}});
      fields(new C({}, excess), {}); json.call(real, excess); equal(reads, 0);
    });
    test('subclasses preserve brand and NewTarget prototype', () => {
      class Derived extends C {}
      const c = new Derived({matrix: 'rgb'}); equal(Object.getPrototypeOf(c), Derived.prototype); fields(c, {matrix: 'rgb'});
      function Target() {} Target.prototype = {author: true};
      const custom = Reflect.construct(C, [{transfer: 'pq'}], Target);
      equal(Object.getPrototypeOf(custom), Target.prototype); fields(custom, {transfer: 'pq'});
    });
    test('non-object NewTarget prototype uses its realm interface prototype', () => {
      const foreign = realms[realms.length - 1], newTarget = foreign.Function(''); newTarget.prototype = null;
      const fallback = Reflect.construct(C, [], newTarget); equal(Object.getPrototypeOf(fallback), foreign.VideoColorSpace.prototype);
      fields(fallback, {});
    }, 'standard-newtarget');
    test('nonserializable platform object rejects clone and storage', () => {
      const c = new C({matrix: 'rgb'});
      throws(globalThis, 'DataCloneError', () => structuredClone(c));
      throws(globalThis, 'DataCloneError', () => structuredClone({c}));
      if (typeof history !== 'undefined') throws(globalThis, 'DataCloneError', () => history.replaceState(c, ''));
    });
    if (realms.length > 1) test('cross realm getters accept genuine objects', () => {
      const other = realms[1 - index], c = new other.VideoColorSpace({primaries: 'bt709', fullRange: true});
      fields(c, {primaries: 'bt709', fullRange: true});
    });
    if (realms.length > 1) test('toJSON creates a dictionary in the method realm', () => {
      const other = realms[1 - index], c = new other.VideoColorSpace({matrix: 'rgb'});
      const result = json.call(c);
      equal(Object.getPrototypeOf(result), realm.Object.prototype);
      equal(Object.getPrototypeOf(result) === other.Object.prototype, false);
      equal(result.matrix, 'rgb');
    }, 'standard-realm');
    test('JSON allocation uses intrinsic Object prototype', () => {
      const saved = realm.Object, c = new C({matrix: 'rgb'});
      try {
        realm.Object = function AuthorObject() {throw Error('global Object override');};
        const result = json.call(c); equal(saved.getPrototypeOf(result), saved.prototype); equal(result.matrix, 'rgb');
      } finally {realm.Object = saved;}
    });
    test('all 756 valid or omitted field combinations', () => {
      let count = 0;
      for (const p of [undefined, ...values.primaries]) for (const t of [undefined, ...values.transfer])
        for (const m of [undefined, ...values.matrix]) for (const f of [undefined, true, false]) {
          const init = {};
          for (const [name, value] of [['primaries', p], ['transfer', t], ['matrix', m], ['fullRange', f]]) if (value !== undefined) init[name] = value;
          fields(new C(init), init); count++;
        }
      equal(count, 756);
    });
  }
  globalThis.__uiEventResults = {complete: true, checks, traces, passed: checks.filter(row => row.passed).length, total: checks.length, combinationsPerRealm: 756};
  return true;
})()
