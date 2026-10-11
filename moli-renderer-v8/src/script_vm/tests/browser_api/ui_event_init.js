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
  const texts = ['', '\ud800', '\udc00', '\ud83d\ude00', 'a\ud800b\udc00\u0000\ud83d\ude00'];
  const snapshot = event => JSON.stringify([
    event.type, event.bubbles, event.cancelable, event.composed, event.defaultPrevented,
    event.detail, event.which, event.data, event.timeStamp
  ]);
  for (const [name, method] of [['UIEvent', 'initUIEvent'], ['CompositionEvent', 'initCompositionEvent']]) {
    const C = globalThis[name], remote = other[name];
    const order = ['bubbles', 'cancelable', 'composed', 'detail', 'view', 'which',
      ...(name === 'CompositionEvent' ? ['data'] : [])];
    const legacy = (view, payload) => {
      const e = new C('before');
      e[method]('after', false, false, view, payload);
      return e;
    };
    check(`${name} defaults`, () => {
      const e = new C('x');
      return e.view === null && e.detail === 0 && e.which === 0 && !e.bubbles && !e.cancelable &&
        !e.composed && !e.isTrusted && (name !== 'CompositionEvent' || e.data === '');
    });
    check(`${name} inherits UIEvent and Event`, () => {
      const e = new C('x');
      return e instanceof UIEvent && e instanceof Event;
    });
    check(`${name} legacy omitted view`, () => {
      const e = new C('before', {view: window, detail: 9, data: 'before'});
      return e[method]('after') === undefined && e.type === 'after' && e.view === null && e.detail === 0 &&
        (name !== 'CompositionEvent' || e.data === '');
    });
    check(`${name} constructor requires new and type`, () => typeError(() => C('x')) && typeError(() => new C()));
    check(`${name} legacy requires type`, () => typeError(() => new C('x')[method]()));
    for (const value of [undefined, null]) {
      check(`${name} nullish dictionary ${String(value)}`, () => new C('x', value).view === null);
    }
    for (const value of [false, 1, '', Symbol(), 1n]) {
      check(`${name} rejects ${typeof value} dictionary`, () => typeError(() => new C('x', value)));
    }
    for (const [value, expected] of [
      [4.9, 4], [-4.9, -4], [2147483648, -2147483648], [4294967297, 1],
      [NaN, 0], [Infinity, 0], [-Infinity, 0], [null, 0], ['17.9', 17]
    ]) {
      check(`${name} detail long ${String(value)}`, () => new C('x', {detail: value}).detail === expected);
      if (name === 'UIEvent') check(`legacy detail long ${String(value)}`, () => legacy(null, value).detail === expected);
    }
    for (const [value, expected] of [[-1, 4294967295], [4294967297, 1], [1.9, 1], [NaN, 0], [Infinity, 0]]) {
      check(`${name} which unsigned long ${String(value)}`, () => new C('x', {which: value}).which === expected);
    }
    for (const field of ['detail', 'which']) {
      for (const value of [Symbol(), 1n]) {
        check(`${name} ${field} rejects ${typeof value}`, () => typeError(() => new C('x', {[field]: value})));
      }
    }
    check(`${name} base-first dictionary order`, () => {
      const seen = [];
      new C('x', new Proxy({}, {get(_target, key) {seen.push(key);}}));
      observed[name + 'DictionaryReads'] = seen;
      return seen.join() === order.join();
    });
    for (const stop of order) {
      check(`${name} getter exception stops at ${stop}`, () => {
        const seen = [], sentinel = {};
        return throws(() => new C('x', new Proxy({}, {get(_target, key) {
          seen.push(key); if (key === stop) throw sentinel;
        }})), sentinel) && seen.join() === order.slice(0, order.indexOf(stop) + 1).join();
      });
    }
    for (const field of ['detail', 'which', ...(name === 'CompositionEvent' ? ['data'] : [])]) {
      check(`${name} ${field} conversion exception stops`, () => {
        const seen = [], sentinel = {};
        return throws(() => new C('x', new Proxy({}, {get(_target, key) {
          seen.push(key); if (key === field) return {[Symbol.toPrimitive]() {throw sentinel;}};
        }})), sentinel) && seen.join() === order.slice(0, order.indexOf(field) + 1).join();
      });
    }
    check(`${name} type converts before dictionary`, () => {
      const seen = [];
      const e = new C({toString() {seen.push('type'); return texts[4];}},
        new Proxy({}, {get(_target, key) {seen.push(key);}}));
      return e.type === texts[4] && seen.join() === ['type', ...order].join();
    });
    check(`${name} inherited dictionary members`, () => {
      const e = new C('x', Object.create({bubbles: true, composed: true, view: other, detail: 3.9, data: texts[4]}));
      return e.view === other && e.detail === 3 && e.bubbles && e.composed &&
        (name !== 'CompositionEvent' || e.data === texts[4]);
    });
    check(`${name} accepts callable dictionary`, () => {
      const init = Object.assign(() => {}, {view: other, detail: 3.9});
      return new C('x', init).view === other && new C('x', init).detail === 3;
    });
    for (const [label, view] of [['window', window], ['child', other], ['null', null], ['undefined', undefined]]) {
      const expected = view === undefined ? null : view;
      check(`${name} constructor accepts ${label} view`, () => new C('x', {view}).view === expected);
      check(`${name} legacy accepts ${label} view`, () => legacy(view).view === expected);
    }
    let traps = 0;
    const handler = {get() {traps++; throw new Error('get trap');}, getPrototypeOf() {traps++; throw new Error('prototype trap');}};
    const revoked = Proxy.revocable(window, handler); revoked.revoke();
    for (const [label, view] of [
      ['ordinary', {}], ['prototype', Object.create(window)], ['proxy', new Proxy(window, handler)],
      ['revoked', revoked.proxy], ['number', 1]
    ]) {
      check(`${name} constructor rejects ${label} view before later conversion`, () => {
        let converted = false;
        return typeError(() => new C('x', {view, which: {valueOf() {converted = true; return 0;}}})) && !converted && traps === 0;
      });
      check(`${name} legacy rejects ${label} view atomically`, () => {
        const e = new C('before', {view: other, detail: 7, data: 'before', cancelable: true});
        e.preventDefault(); const before = snapshot(e); let converted = false;
        const payload = {[Symbol.toPrimitive]() {converted = true; return 4;}};
        return typeError(() => e[method]('after', true, true, view, payload)) &&
          snapshot(e) === before && e.view === other && !converted && traps === 0;
      });
    }
    check(`${name} constructor TypeError belongs to callee realm`, () => typeError(() => new remote('x', {view: {}}), other));
    check(`${name} borrowed legacy accepts genuine other-realm event`, () => {
      const e = new remote('before');
      C.prototype[method].call(e, 'after', true, false, window, name === 'UIEvent' ? 4.9 : texts[4]);
      return e.type === 'after' && e.view === window && e.bubbles &&
        (name === 'UIEvent' ? e.detail === 4 : e.data === texts[4]);
    });
    check(`${name} legacy argument TypeError belongs to function realm`, () => {
      const e = new C('before');
      return typeError(() => remote.prototype[method].call(e, 'after', false, false, {}), other) && e.type === 'before';
    });
    const real = new C('before'), badProxy = Proxy.revocable(real, handler); badProxy.revoke();
    for (const [label, receiver] of [
      ['ordinary', {}], ['prototype', Object.create(real)], ['proxy', new Proxy(real, handler)], ['revoked', badProxy.proxy]
    ]) {
      check(`${name} ${label} receiver rejects before conversion in callee realm`, () => {
        let converted = false;
        const type = {toString() {converted = true; return 'after';}};
        return typeError(() => remote.prototype[method].call(receiver, type), other) && !converted && traps === 0;
      });
    }
    for (const payload of [Symbol(), {[Symbol.toPrimitive]() {throw 19;}}]) {
      check(`${name} legacy payload error retains state ${typeof payload}`, () => {
        const e = new C('before', {view: other, detail: 5, data: 'before', cancelable: true});
        e.preventDefault(); const before = snapshot(e);
        const failed = typeof payload === 'symbol' ? typeError(() => e[method]('after', true, true, null, payload)) :
          throws(() => e[method]('after', true, true, null, payload), 19);
        return failed && snapshot(e) === before && e.view === other;
      });
    }
    check(`${name} legacy successful init preserves creation state`, () => {
      const e = new C('before', {view: other, detail: 5, which: 8, data: 'before', composed: true, cancelable: true});
      const stamp = e.timeStamp, which = e.which; e.preventDefault(); e.stopImmediatePropagation();
      e[method]('after', true, false, null, name === 'UIEvent' ? 4.9 : texts[4]);
      const target = document.createElement('div'); let fired = 0;
      target.addEventListener('after', () => fired++); target.dispatchEvent(e);
      return e.type === 'after' && e.bubbles && !e.cancelable && !e.defaultPrevented && e.composed &&
        e.timeStamp === stamp && e.which === which && fired === 1 && e.view === null &&
        (name === 'UIEvent' ? e.detail === 4 : e.detail === 0 && e.data === texts[4]);
    });
    check(`${name} dispatch guard runs after argument conversion`, () => {
      const e = new C('before', {view: other, detail: 7, data: 'before'});
      const target = document.createElement('div'); const before = snapshot(e), seen = []; let valid = false;
      target.addEventListener('before', () => {
        e[method]({toString() {seen.push('type'); return 'after';}}, false, false, window,
          {[Symbol.toPrimitive](hint) {seen.push(hint); return name === 'UIEvent' ? 3 : 'after';}});
        valid = snapshot(e) === before && e.view === other;
      });
      target.dispatchEvent(e);
      return valid && seen.join() === (name === 'UIEvent' ? 'type,number' : 'type,string');
    });
    check(`${name} createEvent default view remains null`, () => {
      const e = document.createEvent(name); e[method]('after');
      return e.view === null && e.detail === 0 && e.type === 'after';
    });
  }
  for (let i = 0; i < texts.length; i++) {
    const text = texts[i];
    check(`composition constructor data UTF-16 ${i}`, () => new CompositionEvent('x', {data: text}).data === text);
    check(`composition legacy data UTF-16 ${i}`, () => {
      const e = new CompositionEvent('x'); e.initCompositionEvent('after', false, false, null, text);
      return e.data === text;
    });
  }
  for (const [value, expected] of [[undefined, ''], [null, 'null'], [23, '23'], [true, 'true']]) {
    check(`composition data ${String(value)}`, () => new CompositionEvent('x', {data: value}).data === expected);
    check(`composition legacy data ${String(value)}`, () => {
      const e = new CompositionEvent('x'); e.initCompositionEvent('after', false, false, null, value);
      return e.data === expected;
    });
  }
  check('composition data string hint once', () => {
    let calls = 0;
    const value = {[Symbol.toPrimitive](hint) {if (hint !== 'string') throw new Error(hint); calls++; return texts[4];}};
    return new CompositionEvent('x', {data: value}).data === texts[4] && calls === 1;
  });
  check('composition data Symbol rejects', () => typeError(() => new CompositionEvent('x', {data: Symbol()})));
  check('borrowed initUIEvent preserves composition payload', () => {
    const e = new CompositionEvent('before', {data: texts[4], composed: true});
    UIEvent.prototype.initUIEvent.call(e, 'after', true, false, other, 3.9);
    return e.type === 'after' && e.view === other && e.detail === 3 && e.data === texts[4] && e.composed;
  });
  check('borrowed initUIEvent preserves keyboard payload and modifiers', () => {
    const e = new KeyboardEvent('before', {key: texts[4], code: 'KeyA', ctrlKey: true, which: 5, composed: true});
    const which = e.which, key = e.key;
    UIEvent.prototype.initUIEvent.call(e, 'after', false, false, other, 3.9);
    return e.key === key && e.code === 'KeyA' && e.ctrlKey && e.composed && e.which === which && e.view === other && e.detail === 3;
  });
  check('initCompositionEvent rejects UIEvent before type conversion', () => {
    let converted = false;
    return typeError(() => CompositionEvent.prototype.initCompositionEvent.call(new UIEvent('x'),
      {toString() {converted = true; return 'after';}})) && !converted;
  });
  globalThis.__uiEventResults = {total: rows.length, passed: rows.filter(row => row.passed).length, rows, observed};
  return rows.every(row => row.passed);
})()
