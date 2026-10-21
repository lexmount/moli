(() => {
  const rows = [];
  const check = (name, body) => {
    try { rows.push({name, passed: body() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const raises = (body, expected) => {
    try { body(); return false; } catch (error) { return error === expected; }
  };
  const typeError = (body, realm) => {
    try { body(); return false; } catch (error) { return error instanceof realm.TypeError; }
  };
  const child = document.querySelector('#child').contentWindow;
  const realms = [['top', globalThis], ['child', child]];
  for (const [realmName, realm] of realms) {
    const C = realm.CustomEvent;
    const prototype = C.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, 'detail');
    const getter = descriptor?.get;
    const init = prototype.initCustomEvent;
    const prefix = `${realmName}: `;
    const test = (name, body) => check(prefix + name, body);
    test('prototype readonly detail descriptor', () => !!descriptor &&
      descriptor.enumerable && descriptor.configurable && descriptor.set === undefined &&
      !own(descriptor, 'value') && !own(descriptor, 'writable'));
    test('detail getter metadata and realm', () => typeof getter === 'function' &&
      getter.name === 'get detail' && getter.length === 0 &&
      Object.getPrototypeOf(getter) === realm.Function.prototype);
    test('constructor metadata', () => C.length === 1 && C.name === 'CustomEvent');
    test('constructor requires new and type', () =>
      typeError(() => C('x'), realm) && typeError(() => new C(), realm));
    for (const [label, args] of [['omitted', []], ['undefined', [undefined]],
      ['null', [null]], ['empty', [{}]], ['undefined detail', [{detail: undefined}]]]) {
      test(`defaults ${label}`, () => {
        const event = new C('x', ...args);
        return event.detail === null && !event.bubbles && !event.cancelable && !event.composed;
      });
      test(`detail is inherited ${label}`, () => !own(new C('x', ...args), 'detail'));
    }
    for (const [label, value] of [['false', false], ['zero', 0], ['negative zero', -0],
      ['NaN', NaN], ['Infinity', Infinity], ['empty string', ''], ['surrogate', '\ud800'],
      ['symbol', Symbol('payload')], ['bigint', 123n], ['object', {}], ['array', []],
      ['function', () => 1], ['node', realm.document.createElement('div')],
      ['foreign node', document.createElement('div')], ['typed array', new Uint8Array([1, 2])]]) {
      test(`constructor preserves any identity ${label}`, () =>
        Object.is(new C('x', {detail: value}).detail, value));
      test(`legacy initializer preserves any identity ${label}`, () => {
        const event = new C('before');
        init.call(event, 'after', true, true, value);
        return Object.is(event.detail, value) && event.type === 'after' &&
          event.bubbles && event.cancelable;
      });
      test(`borrowed getter preserves any identity ${label}`, () =>
        Object.is(getter.call(new C('x', {detail: value})), value));
    }
    test('any author Proxy and revoked Proxy are payloads without traps', () => {
      let traps = 0;
      const handler = {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 2;}};
      const proxy = new Proxy({}, handler);
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      const a = new C('x', {detail: proxy});
      const b = new C('x', {detail: revoked.proxy});
      return a.detail === proxy && b.detail === revoked.proxy && traps === 0;
    });
    test('detail is readonly without coercing assigned value', () => {
      const value = {}, event = new C('x', {detail: value});
      let converted = 0;
      const assigned = {toString() {converted++; return 'changed';}};
      const strictSet = realm.Function('event', 'value', '"use strict"; event.detail = value;');
      return Reflect.set(event, 'detail', assigned) === false &&
        typeError(() => strictSet(event, assigned), realm) &&
        event.detail === value && converted === 0;
    });
    test('own shadow and frozen wrapper do not replace native detail', () => {
      const first = {}, next = {}, event = new C('before', {detail: first});
      Object.defineProperty(event, 'detail', {value: 'author', configurable: false});
      Object.freeze(event);
      init.call(event, 'after', true, false, next);
      return event.detail === 'author' && getter.call(event) === next && event.type === 'after';
    });
    test('genuine event remains branded after prototype removal', () => {
      const value = {}, event = new C('x', {detail: value});
      Object.setPrototypeOf(event, null);
      return getter.call(event) === value;
    });
    test('detail can point back to the event', () => {
      const event = new C('x'); init.call(event, 'cycle', false, false, event);
      return getter.call(event) === event && event.detail === event;
    });
    for (const [label, options] of [['number', 1], ['zero', 0], ['boolean', true],
      ['false', false], ['string', 'abc'], ['empty string', ''], ['symbol', Symbol()], ['bigint', 1n]]) {
      test(`dictionary rejects primitive ${label}`, () => typeError(() => new C('x', options), realm));
    }
    test('dictionary gets inherited members once in base-first lexical order', () => {
      const reads = [], value = {}, inherited = {};
      for (const name of ['detail', 'composed', 'cancelable', 'bubbles']) {
        Object.defineProperty(inherited, name, {get() {
          reads.push(name); return name === 'detail' ? value : true;
        }});
      }
      const type = {toString() {reads.push('type'); return 'x';}};
      const event = new C(type, Object.create(inherited));
      return reads.join(',') === 'type,bubbles,cancelable,composed,detail' &&
        event.bubbles && event.cancelable && event.composed && event.detail === value;
    });
    test('dictionary uses Get without ownKeys or HasProperty', () => {
      const reads = [], value = {};
      const options = new Proxy({detail: value}, {
        get(target, key) {reads.push(String(key)); return target[key];},
        has() {throw new Error('has trap');}, ownKeys() {throw new Error('ownKeys trap');}
      });
      const event = new C('x', options);
      return reads.join(',') === 'bubbles,cancelable,composed,detail' && event.detail === value;
    });
    test('callable dictionary is accepted', () => {
      const options = () => {}; options.detail = options;
      return new C('x', options).detail === options;
    });
    test('boolean conversion does not coerce objects', () => {
      let conversions = 0;
      const value = {[Symbol.toPrimitive]() {conversions++; throw 1;}};
      const event = new C('x', {bubbles: value, cancelable: value, composed: value, detail: value});
      return event.bubbles && event.cancelable && event.composed &&
        event.detail === value && conversions === 0;
    });
    for (const failedMember of ['bubbles', 'cancelable', 'composed', 'detail']) {
      test(`dictionary stops on abrupt ${failedMember}`, () => {
        const marker = {}, reads = [], options = {};
        const names = ['bubbles', 'cancelable', 'composed', 'detail'];
        for (const name of names) Object.defineProperty(options, name, {get() {
          reads.push(name); if (name === failedMember) throw marker; return true;
        }});
        return raises(() => new C('x', options), marker) &&
          reads.join(',') === names.slice(0, names.indexOf(failedMember) + 1).join(',');
      });
    }
    test('type conversion stops before dictionary access', () => {
      let reads = 0; const marker = {};
      const type = {[Symbol.toPrimitive]() {throw marker;}};
      const options = new Proxy({}, {get() {reads++;}});
      return raises(() => new C(type, options), marker) && reads === 0;
    });
    test('revoked dictionary throws in constructor realm', () => {
      const value = Proxy.revocable({}, {}); value.revoke();
      return typeError(() => new C('x', value.proxy), realm);
    });
    for (const [ownerName, owner] of realms) {
      const event = new owner.CustomEvent('x');
      const revocable = Proxy.revocable(event, {}); revocable.revoke();
      let traps = 0;
      const proxy = new Proxy(event, {get() {traps++;}, getPrototypeOf() {traps++;}, has() {traps++;}});
      const invalid = [['plain', {}], ['prototype', owner.CustomEvent.prototype],
        ['forged prototype', Object.create(owner.CustomEvent.prototype)],
        ['inherited instance', Object.create(event)], ['author Proxy', proxy],
        ['revoked Proxy', revocable.proxy], ['Event', new owner.Event('x')],
        ['UIEvent', new owner.UIEvent('x')], ['null', null], ['undefined', undefined],
        ['number', 1], ['string', 'event']];
      for (const [label, receiver] of invalid) {
        test(`getter rejects ${ownerName} ${label} in callee realm`, () =>
          typeError(() => getter.call(receiver), realm));
        test(`legacy receiver rejects ${ownerName} ${label} before conversion`, () => {
          let conversions = 0;
          const type = {toString() {conversions++; return 'converted';}};
          return typeError(() => init.call(receiver, type), realm) && conversions === 0;
        });
      }
      test(`invalid ${ownerName} receivers do not execute traps`, () => traps === 0);
      test(`borrowed getter and legacy initializer accept ${ownerName} genuine event`, () => {
        const value = {}; init.call(event, 'borrowed', true, false, value);
        return getter.call(event) === value && event.type === 'borrowed' && !own(event, 'detail');
      });
    }
    for (const [label, doc] of [['live', realm.document],
      ['windowless', realm.document.implementation.createHTMLDocument('')]]) {
      test(`createEvent ${label} has inherited default detail and correct realm`, () => {
        const event = doc.createEvent('CustomEvent');
        return event.detail === null && !own(event, 'detail') &&
          Object.getPrototypeOf(event) === prototype && event.type === '';
      });
      test(`createEvent ${label} retains detail across initEvent`, () => {
        const value = {}, event = doc.createEvent('CustomEvent');
        init.call(event, 'custom', true, true, value);
        realm.Event.prototype.initEvent.call(event, 'plain');
        return event.type === 'plain' && event.detail === value && getter.call(event) === value;
      });
    }
    test('legacy arguments default and preserve UTF-16 type', () => {
      const event = new C('x', {detail: 1});
      init.call(event, undefined);
      if (event.type !== 'undefined' || event.detail !== null) return false;
      init.call(event, '\ud800');
      return event.type === '\ud800' && !event.bubbles && !event.cancelable && event.detail === null;
    });
    test('legacy missing type throws and preserves state', () => {
      const event = new C('before', {detail: 1});
      return typeError(() => init.call(event), realm) && event.type === 'before' && event.detail === 1;
    });
    test('legacy abrupt type conversion preserves native state', () => {
      const marker = {}, event = new C('before', {detail: marker});
      return raises(() => init.call(event, {toString() {throw marker;}}), marker) &&
        event.type === 'before' && event.detail === marker;
    });
    test('legacy dispatch guard follows argument conversion', () => {
      const value = {}, replacement = {}, event = new C('before', {composed: true, detail: value});
      const target = realm.document.createElement('div');
      let conversions = 0, during = false;
      target.addEventListener('before', () => {
        init.call(event, {toString() {conversions++; return 'after';}}, true, true, replacement);
        during = event.type === 'before' && getter.call(event) === value;
      });
      target.dispatchEvent(event);
      init.call(event, 'after', true, true, replacement);
      return during && conversions === 1 && event.type === 'after' &&
        getter.call(event) === replacement && event.composed;
    });
    test('prototype deletion does not create an instance detail property', () => {
      try {
        delete prototype.detail;
        const event = new C('x', {detail: 7});
        const legacy = realm.document.createEvent('CustomEvent');
        return !own(event, 'detail') && !('detail' in event) && getter.call(event) === 7 &&
          !own(legacy, 'detail') && getter.call(legacy) === null;
      } finally { if (descriptor) Object.defineProperty(prototype, 'detail', descriptor); }
    });
    test('prototype replacement does not affect private initialization', () => {
      let author = 0;
      try {
        Object.defineProperty(prototype, 'detail', {configurable: true, get() {author++; return 'author';}});
        const event = new C('x', {detail: 9});
        init.call(event, 'after', false, false, 10);
        return !own(event, 'detail') && getter.call(event) === 10 && author === 0 &&
          event.detail === 'author' && author === 1;
      } finally {
        delete prototype.detail;
        if (descriptor) Object.defineProperty(prototype, 'detail', descriptor);
      }
    });
    test('createEvent uses protected constructor', () => {
      const saved = realm.CustomEvent;
      try {
        realm.CustomEvent = function() {throw new Error('author constructor');};
        const event = realm.document.createEvent('CustomEvent');
        return Object.getPrototypeOf(event) === prototype && event.detail === null && !own(event, 'detail');
      } finally { realm.CustomEvent = saved; }
    });
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
