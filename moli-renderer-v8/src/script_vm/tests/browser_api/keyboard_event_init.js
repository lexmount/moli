(() => {
  const rows = [], observed = {};
  const check = (name, run) => {
    try {
      if (run() !== true) throw new Error('assertion failed');
      rows.push({name, passed: true});
    } catch (error) {
      rows.push({name, passed: false, error: String(error), stack: error.stack});
    }
  };
  const typeError = (run, realm = globalThis) => {
    try { run(); } catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const throws = (run, sentinel) => {
    try { run(); } catch (error) { return error === sentinel; }
    return false;
  };
  const other = document.getElementById('child').contentWindow;
  const legacy = (key, location = 0, view = null) => {
    const event = new KeyboardEvent('before');
    event.initKeyboardEvent('after', false, false, view, key, location);
    return event;
  };
  const snapshot = event => JSON.stringify([
    event.type, event.bubbles, event.cancelable, event.composed, event.defaultPrevented,
    event.detail, event.key, event.code, event.location, event.which, event.charCode,
    event.keyCode, event.repeat, event.isComposing, event.ctrlKey, event.altKey,
    event.shiftKey, event.metaKey, event.getModifierState('AltGraph')
  ]);
  const texts = ['', '\ud800', '\udc00', '\ud83d\ude00', 'a\ud800b\udc00\u0000\ud83d\ude00'];
  for (let i = 0; i < texts.length; i++) {
    const text = texts[i];
    for (const field of ['key', 'code']) {
      check(`constructor ${field} UTF-16 ${i}`, () => new KeyboardEvent('x', {[field]: text})[field] === text);
    }
    check(`legacy key UTF-16 ${i}`, () => legacy(text).key === text);
  }
  for (const [value, expected] of [[undefined, ''], [null, 'null'], [23, '23'], [true, 'true']]) {
    for (const field of ['key', 'code']) {
      check(`constructor ${field} ${String(value)}`, () => new KeyboardEvent('x', {[field]: value})[field] === expected);
    }
    check(`legacy key ${String(value)}`, () => legacy(value).key === expected);
  }
  for (const field of ['key', 'code']) {
    check(`constructor ${field} string hint once`, () => {
      let count = 0;
      const value = {[Symbol.toPrimitive](hint) { if (hint !== 'string') throw new Error(hint); count++; return texts[4]; }};
      return new KeyboardEvent('x', {[field]: value})[field] === texts[4] && count === 1;
    });
    check(`constructor ${field} Symbol rejects`, () => typeError(() => new KeyboardEvent('x', {[field]: Symbol()})));
  }
  check('legacy key string hint once', () => {
    let count = 0;
    const value = {[Symbol.toPrimitive](hint) { if (hint !== 'string') throw new Error(hint); count++; return texts[4]; }};
    return legacy(value).key === texts[4] && count === 1;
  });
  check('legacy Symbol key rejects', () => typeError(() => legacy(Symbol())));
  check('constructor default payload', () => {
    const e = new KeyboardEvent('x');
    return e.view === null && e.detail === 0 && e.key === '' && e.code === '' && e.location === 0 &&
      e.charCode === 0 && e.keyCode === 0 && e.which === 0 && !e.repeat && !e.isComposing && !e.isTrusted;
  });
  for (const value of [undefined, null]) {
    check(`nullish dictionary ${String(value)}`, () => new KeyboardEvent('x', value).view === null);
  }
  for (const value of [false, 1, '', Symbol(), 1n]) {
    check(`invalid dictionary ${typeof value}`, () => typeError(() => new KeyboardEvent('x', value)));
  }
  for (const [field, value, expected] of [
    ['detail', 4.9, 4], ['detail', -4.9, -4], ['detail', 2147483648, -2147483648],
    ['location', -1, 4294967295], ['location', 4294967297, 1], ['location', 1.9, 1],
    ['charCode', -1, 4294967295], ['keyCode', -1, 4294967295], ['which', -1, 4294967295],
  ]) {
    check(`constructor ${field} integer ${value}`, () => new KeyboardEvent('x', {[field]: value})[field] === expected);
  }
  for (const field of ['detail', 'location', 'charCode', 'keyCode', 'which']) {
    for (const value of [NaN, Infinity, -Infinity]) {
      check(`constructor ${field} nonfinite ${value}`, () => new KeyboardEvent('x', {[field]: value})[field] === 0);
    }
    for (const value of [1n, Symbol()]) {
      check(`constructor ${field} rejects ${typeof value}`, () => typeError(() => new KeyboardEvent('x', {[field]: value})));
    }
  }
  for (const [value, expected] of [[-1, 4294967295], [4294967297, 1], [1.9, 1], [NaN, 0], [Infinity, 0]]) {
    check(`legacy location integer ${value}`, () => legacy('', value).location === expected);
  }
  check('legacy BigInt location rejects', () => typeError(() => legacy('', 1n)));
  const modifierMembers = ['altKey', 'ctrlKey', 'metaKey', 'modifierAltGraph', 'modifierCapsLock',
    'modifierFn', 'modifierFnLock', 'modifierHyper', 'modifierNumLock', 'modifierScrollLock',
    'modifierSuper', 'modifierSymbol', 'modifierSymbolLock', 'shiftKey'];
  const order = ['bubbles', 'cancelable', 'composed', 'detail', 'view', 'which', ...modifierMembers,
    'charCode', 'code', 'isComposing', 'key', 'keyCode', 'location', 'repeat'];
  check('complete inherited dictionary order', () => {
    const seen = [];
    new KeyboardEvent('x', new Proxy({}, {get(_target, name) { seen.push(name); }}));
    observed.dictionaryReads = seen;
    return seen.join() === order.join();
  });
  for (const stop of order) {
    check(`dictionary getter exception stops at ${stop}`, () => {
      const sentinel = {};
      const seen = [];
      const init = new Proxy({}, {get(_target, name) { seen.push(name); if (name === stop) throw sentinel; }});
      return throws(() => new KeyboardEvent('x', init), sentinel) && seen.join() === order.slice(0, order.indexOf(stop) + 1).join();
    });
  }
  for (const field of ['detail', 'which', 'code', 'key', 'charCode', 'keyCode', 'location']) {
    check(`constructor ${field} conversion exception stops`, () => {
      const sentinel = {}, seen = [];
      const init = new Proxy({}, {get(_target, name) {
        seen.push(name);
        if (name === field) return {[Symbol.toPrimitive]() { throw sentinel; }};
      }});
      return throws(() => new KeyboardEvent('x', init), sentinel) && seen.join() === order.slice(0, order.indexOf(field) + 1).join();
    });
  }
  check('constructor type converts before dictionary', () => {
    const seen = [];
    const type = {toString() {seen.push('type'); return texts[4];}};
    const e = new KeyboardEvent(type, new Proxy({}, {get(_target, name) {seen.push(name);}}));
    return e.type === texts[4] && seen.join() === ['type', ...order].join();
  });
  check('constructor inherits dictionary members', () => {
    const e = new KeyboardEvent('x', Object.create({key: texts[4], code: texts[3], location: -1, view: other, ctrlKey: true}));
    return e.key === texts[4] && e.code === texts[3] && e.location === 4294967295 && e.view === other && e.getModifierState('Control');
  });
  const views = [['window', window], ['child', other], ['null', null], ['undefined', undefined]];
  for (const [name, view] of views) {
    const expected = view === undefined ? null : view;
    check(`constructor accepts ${name} view`, () => new KeyboardEvent('x', {view}).view === expected);
    check(`legacy accepts ${name} view`, () => legacy('', 0, view).view === expected);
  }
  let traps = 0;
  const handler = {get() {traps++; throw new Error('get trap');}, getPrototypeOf() {traps++; throw new Error('prototype trap');}};
  const revoked = Proxy.revocable(window, handler); revoked.revoke();
  for (const [name, view] of [['ordinary', {}], ['prototype', Object.create(window)], ['proxy', new Proxy(window, handler)], ['revoked', revoked.proxy], ['number', 1]]) {
    check(`constructor rejects ${name} view before later conversion`, () => {
      let converted = false;
      const result = typeError(() => new KeyboardEvent('x', {view, code: {toString() {converted = true; return '';}}}));
      return result && !converted && traps === 0;
    });
    check(`legacy rejects ${name} view before key conversion`, () => {
      let converted = false;
      const result = typeError(() => legacy({toString() {converted = true; return '';}}, 0, view));
      return result && !converted && traps === 0;
    });
  }
  check('legacy method descriptor', () => {
    const d = Object.getOwnPropertyDescriptor(KeyboardEvent.prototype, 'initKeyboardEvent');
    return d.value.name === 'initKeyboardEvent' && d.value.length === 1 && d.enumerable && d.configurable && d.writable;
  });
  check('legacy missing required type', () => typeError(() => new KeyboardEvent('x').initKeyboardEvent()));
  check('legacy modifier booleans do not invoke author conversion', () => {
    const e = new KeyboardEvent('x', {modifierAltGraph: true, modifierCapsLock: true});
    const truthy = {[Symbol.toPrimitive]() {throw new Error('boolean conversion');}};
    e.initKeyboardEvent('y', false, false, null, '', 0, truthy, false, truthy, false);
    return e.ctrlKey && !e.altKey && e.shiftKey && !e.metaKey && e.getModifierState('Control') &&
      e.getModifierState('Shift') && !e.getModifierState('AltGraph') && !e.getModifierState('CapsLock');
  });
  check('legacy payload defaults preserve unrelated code and composing state', () => {
    const e = new KeyboardEvent('before', {code:'KeyB', isComposing:true, repeat:true, composed:true, key:'old', view:window, detail:7, keyCode:8, charCode:9, which:10});
    const result = e.initKeyboardEvent('after');
    return result === undefined && e.type === 'after' && e.view === null && e.detail === 0 && e.key === '' &&
      e.code === 'KeyB' && e.isComposing && !e.repeat && e.location === 0 && e.keyCode === 8 && e.charCode === 9 && e.which === 10 && e.composed;
  });
  check('legacy arguments convert left to right before mutation', () => {
    const e = new KeyboardEvent('before', {key:'old', location:3});
    const old = snapshot(e), seen = [], sentinel = {};
    const key = {toString() {seen.push('key'); return 'new';}};
    const location = {valueOf() {seen.push('location'); throw sentinel;}};
    return throws(() => e.initKeyboardEvent({toString() {seen.push('type'); return 'new';}}, true, true, null, key, location), sentinel) &&
      seen.join() === 'type,key,location' && snapshot(e) === old;
  });
  check('legacy ignored extra arguments are not converted', () => {
    const e = new KeyboardEvent('before');
    e.initKeyboardEvent('after', false, false, null, '', 0, false, false, false, false, {[Symbol.toPrimitive]() {throw new Error('extra conversion');}});
    return e.type === 'after';
  });
  check('legacy dispatch guard follows conversions and preserves state', () => {
    const e = new KeyboardEvent('before', {key:'old', ctrlKey:true, code:'KeyB', composed:true});
    const before = snapshot(e), target = new EventTarget();
    let converted = false, ignored = false, propagated = false;
    target.addEventListener('before', () => {
      e.initKeyboardEvent('after', false, false, null, {toString() {converted = true; return 'new';}}, 1, false, true);
      ignored = snapshot(e) === before;
      propagated = typeError(() => e.initKeyboardEvent('after', false, false, null, Symbol()));
    });
    return target.dispatchEvent(e) && converted && ignored && propagated && snapshot(e) === before;
  });
  const real = new KeyboardEvent('before');
  const revokedEvent = Proxy.revocable(real, handler); revokedEvent.revoke();
  for (const [name, receiver] of [['ordinary', {}], ['prototype', Object.create(KeyboardEvent.prototype)], ['inherit-real', Object.create(real)], ['proxy', new Proxy(real, handler)], ['revoked', revokedEvent.proxy], ['other-interface', new MouseEvent('x')]]) {
    check(`legacy rejects ${name} receiver before conversion`, () => {
      let converted = false;
      const type = {toString() {converted = true; return 'x';}};
      return typeError(() => KeyboardEvent.prototype.initKeyboardEvent.call(receiver, type)) && !converted && traps === 0;
    });
  }
  check('cross-realm genuine receiver and Window view', () => {
    const e = new other.KeyboardEvent('before');
    KeyboardEvent.prototype.initKeyboardEvent.call(e, 'after', true, false, window, texts[4], -1, true);
    return e.type === 'after' && e.key === texts[4] && e.view === window && e.location === 4294967295 && e.ctrlKey && e instanceof other.KeyboardEvent;
  });
  check('cross-realm method TypeError uses callee realm', () => typeError(() => other.KeyboardEvent.prototype.initKeyboardEvent.call({}, 'x'), other));
  check('cross-realm constructor TypeError uses callee realm', () => typeError(() => new other.KeyboardEvent('x', {view:{}}), other));
  check('cross-realm argument TypeError uses callee realm', () => typeError(() => other.KeyboardEvent.prototype.initKeyboardEvent.call(real, 'x', false, false, null, Symbol()), other));
  check('genuine receiver remains valid after prototype replacement', () => {
    const e = new KeyboardEvent('before');
    Object.setPrototypeOf(e, null);
    KeyboardEvent.prototype.initKeyboardEvent.call(e, 'after');
    return Object.getOwnPropertyDescriptor(Event.prototype, 'type').get.call(e) === 'after';
  });
  globalThis.__keyboardEventResults = {rows, observed, total: rows.length, passed: rows.filter(row => row.passed).length};
  return rows.every(row => row.passed);
})()
