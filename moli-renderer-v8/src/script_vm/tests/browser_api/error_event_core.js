(function errorEventProbe(realms) {
  const rows = [], errors = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const fields = ['message', 'filename', 'lineno', 'colno', 'error'];
  const keys = ['bubbles', 'cancelable', 'composed', 'colno', 'error', 'filename', 'lineno', 'message'];
  const units = text => Array.from({length: text.length}, (_, i) => text.charCodeAt(i));
  const scalar = text => Array.from(text, char => {
    const code = char.codePointAt(0);
    return code >= 0xD800 && code <= 0xDFFF ? '\uFFFD' : char;
  }).join('');
  const unsignedLong = value => {
    const number = Number(value);
    if (!Number.isFinite(number) || number === 0) return 0;
    return ((Math.trunc(number) % 4294967296) + 4294967296) % 4294967296;
  };
  const observed = event => ({type: units(event.type), message: units(event.message),
    filename: units(event.filename), lineno: String(event.lineno), colno: String(event.colno)});
  function outcome(Ctor, args, expected, sentinel) {
    try {
      const event = Reflect.construct(Ctor, args);
      return {checks: {throws: false, identityOrRealm: false}, observed: 'returned', event};
    } catch (error) {
      return {checks: {throws: sentinel ? error === sentinel : error.name === 'TypeError',
        identityOrRealm: sentinel ? error === sentinel : error instanceof expected}, observed: error.name || typeof error};
    }
  }
  try {
    for (const [r, realm] of realms.entries()) {
      const Ctor = realm.ErrorEvent;
      const sample = new Ctor('x');
      const descriptors = fields.map(name => Object.getOwnPropertyDescriptor(Ctor.prototype, name));
      record(`${r}/metadata`, {name: Ctor.name === 'ErrorEvent', length: Ctor.length === 1,
        base: Object.getPrototypeOf(Ctor.prototype) === realm.Event.prototype,
        attributes: descriptors.every(d => typeof d?.get === 'function' && d.set === undefined && d.enumerable && d.configurable),
        inherited: fields.every(name => !Object.hasOwn(sample, name)),
        trusted: sample.isTrusted === false, legacyAbsent: !('initErrorEvent' in sample)});
      for (const [i, init] of [undefined, null, {}, [], function () {}, new realm.Date(0)].entries()) {
        const event = new Ctor('x', init);
        record(`${r}/defaults-${i}`, {message: event.message === '', filename: event.filename === '',
          lineno: event.lineno === 0, colno: event.colno === 0, error: event.error === undefined,
          flags: !event.bubbles && !event.cancelable && !event.composed}, observed(event));
      }
      for (const [i, init] of [0, 1, '', 'init', false, true, Symbol('init'), 1n].entries()) {
        const result = outcome(Ctor, ['x', init], realm.TypeError);
        record(`${r}/invalid-dictionary-${i}`, result.checks, result.observed);
      }
      const texts = ['', 'plain', '\uD800', '\uDC00', '\uD83D\uDE00', 'a\uD800\0\uDC00b',
        '\r\n', null, undefined, false, true, 42, NaN, -0, Infinity, -Infinity, 1n];
      for (const [i, value] of texts.entries()) {
        const marker = {};
        const event = new Ctor('t\uD800', {message: value, filename: value, error: marker});
        const text = value === undefined ? '' : String(value);
        record(`${r}/strings-${i}`, {type: event.type === 't\uD800', message: event.message === text,
          filename: event.filename === scalar(text), anyIdentity: event.error === marker}, observed(event));
      }
      const numbers = [undefined, null, false, true, 0, -0, 1.75, -1.75, -1, 4294967296,
        4294967297, -4294967297, Number.MAX_SAFE_INTEGER, NaN, Infinity, -Infinity, '12.8', '', 'invalid', []];
      for (const [i, value] of numbers.entries()) {
        const event = new Ctor('x', {lineno: value, colno: value});
        const expected = value === undefined ? 0 : unsignedLong(value);
        record(`${r}/unsigned-long-${i}`, {lineno: event.lineno === expected, colno: event.colno === expected,
          normalizedZero: expected !== 0 || (Object.is(event.lineno, 0) && Object.is(event.colno, 0))}, observed(event));
      }
      let anyConversions = 0;
      const opaque = new Proxy({}, {get() {anyConversions++; throw new Error('any trap');}});
      for (const [i, value] of [undefined, null, 1, 'text', 1n, Symbol('any'), opaque, sample].entries()) {
        const event = new Ctor('x', {error: value});
        record(`${r}/any-${i}`, {identity: event.error === value, noConversion: anyConversions === 0}, typeof event.error);
      }
      for (const inherited of [false, true]) {
        const trace = [];
        const marker = {};
        const number = name => ({[Symbol.toPrimitive](hint) {trace.push(`${name}:${hint}`); return -1.75;}});
        const text = name => ({[Symbol.toPrimitive](hint) {trace.push(`${name}:${hint}`); return '\uD800';}});
        const boolean = {[Symbol.toPrimitive]() {throw new Error('boolean conversion');}};
        const values = {bubbles: boolean, cancelable: true, composed: true, colno: number('colno'),
          error: marker, filename: text('filename'), lineno: number('lineno'), message: text('message')};
        const dictionary = new Proxy(Object.create(null), {get(_, name) {trace.push(name); return values[name];},
          ownKeys() {throw new Error('dictionary enumeration');}});
        const init = inherited ? Object.create(dictionary) : dictionary;
        const type = {[Symbol.toPrimitive](hint) {trace.push(`type:${hint}`); return 't\uD800';}};
        const event = new Ctor(type, init);
        const expected = ['type:string', 'bubbles', 'cancelable', 'composed', 'colno', 'colno:number',
          'error', 'filename', 'filename:string', 'lineno', 'lineno:number', 'message', 'message:string'];
        record(`${r}/order-${inherited}`, {order: JSON.stringify(trace) === JSON.stringify(expected),
          flags: event.bubbles && event.cancelable && event.composed,
          values: event.message === '\uD800' && event.filename === '\uFFFD' && event.colno === 4294967295 && event.lineno === 4294967295,
          anyIdentity: event.error === marker}, {trace, values: observed(event)});
      }
      for (const key of keys) {
        const trace = [], sentinel = {};
        const dictionary = new Proxy({}, {get(_, name) {trace.push(name); if (name === key) throw sentinel;}});
        const result = outcome(Ctor, ['x', dictionary], realm.TypeError, sentinel);
        record(`${r}/getter-throws-${key}`, {...result.checks,
          stops: JSON.stringify(trace) === JSON.stringify(keys.slice(0, keys.indexOf(key) + 1))}, {outcome: result.observed, trace});
      }
      for (const key of ['colno', 'filename', 'lineno', 'message']) {
        const trace = [], sentinel = {};
        const dictionary = new Proxy({}, {get(_, name) {trace.push(name); if (name === key) return {
          [Symbol.toPrimitive](hint) {trace.push(`convert:${hint}`); throw sentinel;}};}});
        const result = outcome(Ctor, ['x', dictionary], realm.TypeError, sentinel);
        const expected = [...keys.slice(0, keys.indexOf(key) + 1), `convert:${key === 'colno' || key === 'lineno' ? 'number' : 'string'}`];
        record(`${r}/conversion-throws-${key}`, {...result.checks, stops: JSON.stringify(trace) === JSON.stringify(expected)}, {outcome: result.observed, trace});
      }
      for (const key of ['colno', 'filename', 'lineno', 'message']) {
        for (const [i, value] of (key === 'colno' || key === 'lineno' ? [Symbol('number'), 1n] : [Symbol('string')]).entries()) {
          const trace = [];
          const dictionary = new Proxy({}, {get(_, name) {trace.push(name); if (name === key) return value;}});
          const result = outcome(Ctor, ['x', dictionary], realm.TypeError);
          record(`${r}/conversion-typeerror-${key}-${i}`, {...result.checks,
            stops: JSON.stringify(trace) === JSON.stringify(keys.slice(0, keys.indexOf(key) + 1))}, {outcome: result.observed, trace});
        }
      }
      {
        const trace = [], sentinel = {};
        const type = {[Symbol.toPrimitive]() {trace.push('type'); throw sentinel;}};
        const init = new Proxy({}, {get(_, name) {trace.push(name);}});
        const result = outcome(Ctor, [type, init], realm.TypeError, sentinel);
        record(`${r}/type-before-dictionary`, {...result.checks, noDictionaryReads: trace.join() === 'type'}, trace);
      }
      record(`${r}/symbol-type`, outcome(Ctor, [Symbol('type'), {}], realm.TypeError).checks);
      record(`${r}/missing-type`, outcome(Ctor, [], realm.TypeError).checks);
      {
        let conversions = 0, caught;
        const value = {[Symbol.toPrimitive]() {conversions++; return 'x';}};
        try {Reflect.apply(Ctor, undefined, [value, {}]);} catch (error) {caught = error;}
        record(`${r}/new-required`, {typeError: caught instanceof realm.TypeError, noConversion: conversions === 0}, caught?.name);
      }
      {
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        const result = outcome(Ctor, ['x', revoked.proxy], realm.TypeError);
        record(`${r}/revoked-dictionary`, result.checks, result.observed);
      }
      {
        let conversions = 0;
        const ignored = {[Symbol.toPrimitive]() {conversions++; throw new Error('extra argument');}};
        const event = new Ctor('x', {message: '\uD800'}, ignored);
        record(`${r}/ignored-extra`, {message: event.message === '\uD800', noConversion: conversions === 0});
      }
      {
        class Derived extends Ctor {constructor(...args) {super(...args); this.marker = 7;}}
        const event = new Derived('x', {message: '\uD800', colno: -1});
        record(`${r}/subclass`, {prototype: Object.getPrototypeOf(event) === Derived.prototype,
          brands: event instanceof Ctor && event instanceof realm.Event, marker: event.marker === 7,
          payload: event.message === '\uD800' && event.colno === 4294967295});
        function Alternate() {}
        const alternate = Reflect.construct(Ctor, ['x', {message: '\uD800'}], Alternate);
        const get = Object.getOwnPropertyDescriptor(Ctor.prototype, 'message')?.get;
        const message = typeof get === 'function' ? get.call(alternate) : undefined;
        record(`${r}/alternate-new-target`, {prototype: Object.getPrototypeOf(alternate) === Alternate.prototype,
          nativeBrand: typeof get === 'function' && message === '\uD800', noInventedPrototype: !(alternate instanceof Ctor)});
      }
      for (const [s, source] of realms.entries()) {
        const real = new source.ErrorEvent('x', {message: '\uD800', filename: '\uDC00', colno: -1, error: sample});
        const revoked = Proxy.revocable(real, {}); revoked.revoke();
        let traps = 0;
        const author = new Proxy(real, {get() {traps++; throw new Error('author trap');}});
        const receivers = [null, undefined, {}, new source.Event('x'), Ctor.prototype,
          Object.create(real), Object.create(Ctor.prototype), author, revoked.proxy];
        for (const name of fields) {
          const get = Object.getOwnPropertyDescriptor(Ctor.prototype, name)?.get;
          const expected = {message: '\uD800', filename: '\uFFFD', lineno: 0, colno: 4294967295, error: sample}[name];
          const value = typeof get === 'function' ? Reflect.apply(get, real, []) : real[name];
          record(`${r}/${s}/genuine-getter-${name}`, {present: typeof get === 'function', value: value === expected});
          for (const [i, receiver] of receivers.entries()) {
            let caught;
            if (typeof get === 'function') {try {Reflect.apply(get, receiver, []);} catch (error) {caught = error;}}
            record(`${r}/${s}/invalid-getter-${name}-${i}`, {present: typeof get === 'function', typeError: caught instanceof realm.TypeError,
              noAuthorTrap: traps === 0}, caught?.name || 'returned');
          }
        }
      }
      for (const poison of [false, true]) {
        const saved = realm.onerror, marker = {};
        const event = new Ctor('error', {cancelable: true, message: '\uD800', filename: '\uDC00',
          lineno: -1.75, colno: 4294967305, error: marker});
        let reads = 0, calls = 0, facts = null;
        if (poison) for (const name of fields) Object.defineProperty(event, name,
          {configurable: true, get() {reads++; throw new Error('public payload getter');}});
        realm.onerror = function (message, filename, lineno, colno, error) {
          calls++;
          facts = {length: arguments.length, thisIsGlobal: this === realm,
            message: units(message), filename: units(filename), lineno: String(lineno), colno: String(colno),
            anyIdentity: error === marker};
          return true;
        };
        try {
          const result = realm.dispatchEvent(event);
          record(`${r}/onerror-${poison}`, {calls: calls === 1, length: facts?.length === 5,
            receiver: facts?.thisIsGlobal === true, message: JSON.stringify(facts?.message) === '[55296]',
            filename: JSON.stringify(facts?.filename) === '[65533]', lineno: facts?.lineno === '4294967295',
            colno: facts?.colno === '9', anyIdentity: facts?.anyIdentity === true,
            cancelled: result === false && event.defaultPrevented, noPublicReads: reads === 0}, facts);
        } finally {realm.onerror = saved;}
      }
    }
  } catch (error) {errors.push(String(error.stack || error));}
  return {rows, errors};
})
