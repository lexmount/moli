(realms) => {
  const rows = [], errors = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const attempt = (label, run) => { try { run(); } catch (error) { errors.push({label, error: String(error.stack || error)}); } };
  const units = text => Array.from({length: text.length}, (_, i) => text.charCodeAt(i));
  const scalar = text => text.toWellFormed();
  const unsigned = (value, bits) => { const n = Number(value); if (!Number.isFinite(n) || n === 0) return 0; const modulus = 2 ** bits; return ((Math.trunc(n) % modulus) + modulus) % modulus; };
  const urls = ['blockedURI', 'documentURI', 'referrer', 'sourceFile'];
  const strings = ['effectiveDirective', 'originalPolicy', 'sample', 'violatedDirective'];
  const numbers = ['columnNumber', 'lineNumber', 'statusCode'];
  const fields = [...urls, ...strings, ...numbers, 'disposition'];
  const keys = ['bubbles', 'cancelable', 'composed', 'blockedURI', 'columnNumber', 'disposition', 'documentURI', 'effectiveDirective', 'lineNumber', 'originalPolicy', 'referrer', 'sample', 'sourceFile', 'statusCode', 'violatedDirective'];
  for (const [r, realm] of realms.entries()) {
    const Ctor = realm.SecurityPolicyViolationEvent, prefix = `${r}/SecurityPolicyViolationEvent`;
    attempt(prefix + '/metadata', () => {
      const e = new Ctor('violation');
      record(prefix + '/constructor', {name: Ctor.name === 'SecurityPolicyViolationEvent', length: Ctor.length === 1, parent: Object.getPrototypeOf(Ctor.prototype) === realm.Event.prototype, instance: e instanceof realm.Event});
      for (const field of fields) {
        const d = Object.getOwnPropertyDescriptor(Ctor.prototype, field);
        record(prefix + '/descriptor-' + field, {getter: typeof d?.get === 'function', name: d?.get?.name === 'get ' + field, length: d?.get?.length === 0, enumerable: d?.enumerable === true, configurable: d?.configurable === true, setter: d?.set === undefined, own: !Object.hasOwn(e, field)});
      }
    });
    for (const [i, init] of [undefined, null, {}, [], function() {}, new Date(0)].entries()) attempt(prefix + '/default-' + i, () => {
      const e = new Ctor('violation', init);
      record(prefix + '/default-' + i, {strings: [...urls, ...strings].every(f => e[f] === ''), numbers: numbers.every(f => e[f] === 0), disposition: e.disposition === 'enforce', flags: !e.bubbles && !e.cancelable && !e.composed});
    });
    for (const [i, value] of [1, '', false, Symbol('init'), 1n].entries()) attempt(prefix + '/dictionary-' + i, () => {
      let error; try { new Ctor('x', value); } catch (e) { error = e; }
      record(prefix + '/dictionary-' + i, {typeError: error instanceof realm.TypeError}, error?.name || 'returned');
    });
    const values = [undefined, null, '', 'plain', '\uD800', '\uDC00', '\uD83D\uDE80', 'x\uD800y', '\0', true, false, -3, 3.5, NaN, Infinity, 1n, ['a', 'b']];
    for (const field of [...urls, ...strings]) {
      for (const [i, value] of values.entries()) attempt(prefix + '/string-' + field + '-' + i, () => {
        const raw = value === undefined ? '' : String(value), expected = urls.includes(field) ? scalar(raw) : raw;
        const actual = new Ctor('x', {[field]: value})[field];
        record(prefix + '/string-' + field + '-' + i, {value: actual === expected}, units(actual));
      });
      attempt(prefix + '/coercion-' + field, () => {
        const trace = [], input = {[Symbol.toPrimitive](hint) { trace.push(hint); return '\uD800'; }};
        const actual = new Ctor('x', {[field]: input})[field];
        record(prefix + '/coercion-' + field, {value: actual === (urls.includes(field) ? '\uFFFD' : '\uD800'), hint: trace.length === 1 && trace[0] === 'string'}, {trace, units: units(actual)});
        let error; try { new Ctor('x', {[field]: Symbol('text')}); } catch (e) { error = e; }
        record(prefix + '/symbol-' + field, {typeError: error instanceof realm.TypeError});
      });
    }
    for (const field of numbers) {
      const bits = field === 'statusCode' ? 16 : 32;
      for (const [i, value] of [undefined, null, 0, -0, -1, -3.5, 3.5, 65535, 65536, 2147483648, 4294967295, 4294967296, 4294967297, NaN, Infinity, -Infinity, true, false, '65537', 'NaN', {valueOf() { return -1; }}].entries()) attempt(prefix + '/number-' + field + '-' + i, () => {
        const expected = value === undefined ? 0 : unsigned(value, bits), actual = new Ctor('x', {[field]: value})[field];
        record(prefix + '/number-' + field + '-' + i, {value: actual === expected, positiveZero: !Object.is(actual, -0)}, actual);
      });
      for (const [i, value] of [Symbol('number'), 1n].entries()) attempt(prefix + '/invalid-number-' + field + '-' + i, () => {
        let error; try { new Ctor('x', {[field]: value}); } catch (e) { error = e; }
        record(prefix + '/invalid-number-' + field + '-' + i, {typeError: error instanceof realm.TypeError});
      });
      attempt(prefix + '/number-hint-' + field, () => {
        const trace = [], value = {[Symbol.toPrimitive](hint) { trace.push(hint); return -1; }};
        const actual = new Ctor('x', {[field]: value})[field];
        record(prefix + '/number-hint-' + field, {value: actual === 2 ** bits - 1, hint: trace.length === 1 && trace[0] === 'number'}, {actual, trace});
      });
    }
    for (const [i, value] of ['enforce', 'report', undefined, 'Enforce', '', 'report-only', null, false, 1, Symbol('enum'), 1n, ['report'], {toString() { return 'enforce'; }}].entries()) attempt(prefix + '/enum-' + i, () => {
      let e, error; try { e = new Ctor('x', {disposition: value}); } catch (caught) { error = caught; }
      const valid = [0, 1, 2, 11, 12].includes(i);
      record(prefix + '/enum-' + i, valid ? {value: e?.disposition === (value === undefined ? 'enforce' : String(value))} : {typeError: error instanceof realm.TypeError}, e?.disposition || error?.name || 'returned');
    });
    attempt(prefix + '/order', () => {
      const trace = [];
      new Ctor('x', new Proxy({}, {get(_, key) { trace.push(key); }}));
      record(prefix + '/order', {order: JSON.stringify(trace) === JSON.stringify(keys)}, trace);
      for (const [i, key] of keys.entries()) {
        const sentinel = {}, reads = []; let error;
        try { new Ctor('x', new Proxy({}, {get(_, k) { reads.push(k); if (k === key) throw sentinel; }})); } catch (e) { error = e; }
        record(prefix + '/throw-get-' + key, {identity: error === sentinel, order: JSON.stringify(reads) === JSON.stringify(keys.slice(0, i + 1))}, reads);
      }
      for (const field of [...urls, ...strings, ...numbers, 'disposition']) {
        const sentinel = {}, reads = []; let error;
        try { new Ctor('x', new Proxy({}, {get(_, key) { reads.push(key); return key === field ? {[Symbol.toPrimitive]() { throw sentinel; }} : undefined; }})); } catch (e) { error = e; }
        record(prefix + '/throw-convert-' + field, {identity: error === sentinel, order: JSON.stringify(reads) === JSON.stringify(keys.slice(0, keys.indexOf(field) + 1))}, reads);
      }
      const reads = []; let error;
      try { new Ctor('x', new Proxy({}, {get(_, key) { reads.push(key); return key === 'disposition' ? 'invalid' : undefined; }})); } catch (e) { error = e; }
      record(prefix + '/invalid-enum-stops', {typeError: error instanceof realm.TypeError, order: JSON.stringify(reads) === JSON.stringify(keys.slice(0, keys.indexOf('disposition') + 1))}, reads);
      const init = Object.create({sample: '\uD800', blockedURI: '\uD800', lineNumber: -1});
      init.bubbles = 1; init.cancelable = {}; init.composed = 'yes';
      const event = new Ctor('x', init);
      record(prefix + '/inherited', {sample: event.sample === '\uD800', blockedURI: event.blockedURI === '\uFFFD', line: event.lineNumber === 4294967295, flags: event.bubbles && event.cancelable && event.composed}, {sample: units(event.sample), blockedURI: units(event.blockedURI), line: event.lineNumber});
    });
    attempt(prefix + '/construct', () => {
      let call, missing; try { Ctor('x'); } catch (e) { call = e; } try { new Ctor(); } catch (e) { missing = e; }
      record(prefix + '/required', {call: call instanceof realm.TypeError, missing: missing instanceof realm.TypeError});
      const trace = [], type = {[Symbol.toPrimitive](hint) { trace.push('type:' + hint); return '\uD800'; }};
      const e = new Ctor(type, new Proxy({}, {get(_, key) { trace.push(key); }}));
      record(prefix + '/type-first', {type: e.type === '\uD800', order: JSON.stringify(trace) === JSON.stringify(['type:string', ...keys])}, trace);
      const sentinel = {}, reads = []; let error;
      try { new Ctor({toString() { throw sentinel; }}, new Proxy({}, {get(_, key) { reads.push(key); }})); } catch (e) { error = e; }
      record(prefix + '/type-throw', {identity: error === sentinel, reads: reads.length === 0});
      class Derived extends Ctor {}
      const d = new Derived('x', {sample: '\uD800', lineNumber: -1});
      record(prefix + '/subclass', {identity: d instanceof Derived && d instanceof Ctor, sample: d.sample === '\uD800', line: d.lineNumber === 4294967295}, {sample: units(d.sample), line: d.lineNumber});
      function Alternate() {} Alternate.prototype = Object.create(Ctor.prototype);
      const alternate = Reflect.construct(Ctor, ['x', {sample: '\uD800'}], Alternate);
      record(prefix + '/new-target', {prototype: Object.getPrototypeOf(alternate) === Alternate.prototype, sample: alternate.sample === '\uD800'}, units(alternate.sample));
    });
    for (const field of fields) attempt(prefix + '/receiver-' + field, () => {
      const getter = Object.getOwnPropertyDescriptor(Ctor.prototype, field)?.get;
      if (!getter) { record(prefix + '/receiver-' + field, {getter: false}); return; }
      const genuine = new Ctor('x'), revoked = Proxy.revocable(genuine, {}); revoked.revoke();
      for (const [i, value] of [null, undefined, {}, Object.create(Ctor.prototype), Object.create(genuine), new realm.Event('x'), new Proxy(genuine, {}), revoked.proxy].entries()) {
        let error; try { getter.call(value); } catch (e) { error = e; }
        record(prefix + '/receiver-' + field + '-' + i, {typeError: error instanceof realm.TypeError});
      }
      let traps = 0; const proxy = new Proxy(genuine, {get() { traps++; throw 'get'; }, getPrototypeOf() { traps++; throw 'prototype'; }}); let error;
      try { getter.call(proxy); } catch (e) { error = e; }
      record(prefix + '/receiver-traps-' + field, {typeError: error instanceof realm.TypeError, traps: traps === 0});
      for (const [j, other] of realms.entries()) {
        const value = urls.includes(field) || strings.includes(field) ? '\uD800' : field === 'disposition' ? 'report' : -1;
        const event = new other.SecurityPolicyViolationEvent('x', {[field]: value});
        const expected = urls.includes(field) ? '\uFFFD' : strings.includes(field) ? '\uD800' : field === 'disposition' ? 'report' : field === 'statusCode' ? 65535 : 4294967295;
        const actual = getter.call(event);
        record(prefix + '/foreign-getter-' + field + '-' + j, {value: actual === expected}, typeof actual === 'string' ? units(actual) : actual);
      }
    });
  }
  return {rows, errors};
}
