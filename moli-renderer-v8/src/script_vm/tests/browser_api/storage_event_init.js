(() => {
  const rows = [];
  const check = (name, run) => {
    try {
      if (run() !== true) throw new Error('assertion failed');
      rows.push({name, passed: true});
    } catch (error) {
      rows.push({name, passed: false, error: String(error), stack: error.stack});
    }
  };
  const throws = (run, expected) => {
    try { run(); } catch (error) { return error === expected; }
    return false;
  };
  const typeError = (run, realm = globalThis) => {
    try { run(); } catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const legacy = (key, oldValue, newValue, url, storageArea) => {
    const event = new StorageEvent('before');
    event.initStorageEvent('after', true, true, key, oldValue, newValue, url, storageArea);
    return event;
  };
  const strings = ['', '\ud800', '\udc00', '\ud83d\ude00', 'a\ud800b\udc00\u0000\ud83d\ude00'];
  for (const field of ['key', 'oldValue', 'newValue']) {
    for (let i = 0; i < strings.length; i++) {
      const text = strings[i];
      check(`constructor ${field} UTF-16 ${i}`, () => new StorageEvent('x', {[field]: text})[field] === text);
      check(`legacy ${field} UTF-16 ${i}`, () => legacy(text, text, text, '')[field] === text);
    }
    for (const value of [undefined, null]) {
      check(`constructor ${field} nullable ${String(value)}`, () => new StorageEvent('x', {[field]: value})[field] === null);
      check(`legacy ${field} nullable ${String(value)}`, () => legacy(value, value, value, '')[field] === null);
    }
    check(`constructor ${field} string conversion`, () => {
      let conversions = 0;
      const value = {[Symbol.toPrimitive](hint) { if (hint !== 'string') throw new Error(hint); conversions++; return strings[4]; }};
      const event = new StorageEvent('x', {[field]: value});
      return conversions === 1 && event[field] === strings[4];
    });
    check(`legacy ${field} string conversion`, () => {
      let conversions = 0;
      const value = {toString() { conversions++; return strings[4]; }};
      const values = {key: null, oldValue: null, newValue: null, [field]: value};
      const event = legacy(values.key, values.oldValue, values.newValue, '');
      return conversions === 1 && event[field] === strings[4];
    });
    check(`constructor ${field} Symbol rejects`, () => typeError(() => new StorageEvent('x', {[field]: Symbol()})));
    check(`legacy ${field} Symbol rejects`, () => {
      const values = {key: null, oldValue: null, newValue: null, [field]: Symbol()};
      return typeError(() => legacy(values.key, values.oldValue, values.newValue, ''));
    });
  }
  check('constructor defaults', () => {
    const event = new StorageEvent('x');
    return event.key === null && event.oldValue === null && event.newValue === null && event.url === '' && event.storageArea === null && !event.bubbles && !event.cancelable && !event.composed;
  });
  check('legacy defaults', () => {
    const event = new StorageEvent('before', {key: 'k', oldValue: 'o', newValue: 'n', composed: true});
    const result = event.initStorageEvent('after');
    return result === undefined && event.type === 'after' && event.key === null && event.oldValue === null && event.newValue === null && event.url === '' && event.storageArea === null && event.composed;
  });
  for (const [value, expected] of [[undefined, ''], [null, 'null'], [strings[4], 'a\ufffdb\ufffd\u0000\ud83d\ude00']]) {
    check(`constructor url USVString ${String(value)}`, () => new StorageEvent('x', {url: value}).url === expected);
    check(`legacy url USVString ${String(value)}`, () => legacy(null, null, null, value).url === expected);
  }
  for (const value of [undefined, null]) {
    check(`nullable dictionary ${String(value)}`, () => new StorageEvent('x', value).key === null);
  }
  for (const value of [false, true, 0, 1, '', 'init', Symbol('init'), 1n]) {
    check(`primitive dictionary ${String(value)}`, () => typeError(() => new StorageEvent('x', value)));
  }
  check('function dictionary', () => {
    const init = () => {};
    init.key = strings[4];
    return new StorageEvent('x', init).key === strings[4];
  });
  const order = ['bubbles', 'cancelable', 'composed', 'key', 'newValue', 'oldValue', 'storageArea', 'url'];
  check('inherited then own dictionary order', () => {
    const seen = [];
    new StorageEvent('x', new Proxy({}, {get(_target, name) { seen.push(name); }}));
    return seen.join() === order.join();
  });
  check('dictionary reads and conversions are interleaved once', () => {
    const seen = [];
    const init = new Proxy({}, {get(_target, name) {
      seen.push(`get:${name}`);
      if (['key', 'newValue', 'oldValue', 'url'].includes(name)) {
        return {[Symbol.toPrimitive](hint) { seen.push(`convert:${name}:${hint}`); return strings[4]; }};
      }
    }});
    const event = new StorageEvent('x', init);
    const expected = order.flatMap(name => ['key', 'newValue', 'oldValue', 'url'].includes(name) ? [`get:${name}`, `convert:${name}:string`] : [`get:${name}`]);
    return seen.join() === expected.join() && event.key === strings[4] && event.url === 'a\ufffdb\ufffd\u0000\ud83d\ude00';
  });
  for (const stop of order) {
    check(`dictionary getter exception stops at ${stop}`, () => {
      const sentinel = {};
      const seen = [];
      const init = new Proxy({}, {get(_target, name) { seen.push(name); if (name === stop) throw sentinel; }});
      return throws(() => new StorageEvent('x', init), sentinel) && seen.join() === order.slice(0, order.indexOf(stop) + 1).join();
    });
  }
  for (const stop of ['key', 'newValue', 'oldValue', 'url']) {
    check(`dictionary conversion exception stops at ${stop}`, () => {
      const sentinel = {};
      const seen = [];
      const init = new Proxy({}, {get(_target, name) { seen.push(name); if (name === stop) return {toString() { throw sentinel; }}; }});
      return throws(() => new StorageEvent('x', init), sentinel) && seen.join() === order.slice(0, order.indexOf(stop) + 1).join();
    });
  }
  check('inherited dictionary values retain UTF-16', () => new StorageEvent('x', Object.create({key: strings[4], newValue: strings[1]})).key === strings[4]);
  check('event type converts before dictionary', () => {
    const seen = [];
    const type = {toString() { seen.push('type'); return strings[4]; }};
    const init = new Proxy({}, {get(_target, name) { seen.push(name); }});
    const event = new StorageEvent(type, init);
    return event.type === strings[4] && seen.join() === ['type', ...order].join();
  });
  check('legacy positional conversion order', () => {
    const seen = [];
    const value = name => ({toString() { seen.push(name); return name; }});
    const event = new StorageEvent('before');
    event.initStorageEvent(value('type'), true, true, value('key'), value('oldValue'), value('newValue'), value('url'), null);
    return seen.join() === 'type,key,oldValue,newValue,url' && event.key === 'key' && event.oldValue === 'oldValue' && event.newValue === 'newValue';
  });
  for (const field of ['key', 'oldValue', 'newValue', 'url']) {
    check(`legacy failed ${field} conversion is atomic`, () => {
      const event = new StorageEvent('before', {bubbles: true, composed: true, key: 'k', oldValue: 'o', newValue: 'n', url: 'u'});
      const sentinel = {};
      const seen = [];
      const names = ['key', 'oldValue', 'newValue', 'url'];
      const values = names.map(name => ({toString() { seen.push(name); if (name === field) throw sentinel; return name; }}));
      return throws(() => event.initStorageEvent('after', false, true, ...values, null), sentinel) && seen.join() === names.slice(0, names.indexOf(field) + 1).join() && event.type === 'before' && event.bubbles && !event.cancelable && event.composed && event.key === 'k' && event.oldValue === 'o' && event.newValue === 'n' && event.url === 'u';
    });
  }
  const invalidAreas = [false, 1, 'storage', {}, [], Object.create(Storage.prototype)];
  for (let i = 0; i < invalidAreas.length; i++) {
    const storageArea = invalidAreas[i];
    check(`constructor rejects unbranded storageArea ${i}`, () => typeError(() => new StorageEvent('x', {storageArea})));
    check(`legacy rejects unbranded storageArea ${i} before mutation`, () => {
      const event = new StorageEvent('before', {key: 'keep'});
      return typeError(() => event.initStorageEvent('after', false, false, null, null, null, '', storageArea)) && event.type === 'before' && event.key === 'keep';
    });
  }
  check('invalid storageArea stops before url getter', () => {
    let reads = 0;
    return typeError(() => new StorageEvent('x', {storageArea: {}, get url() { reads++; return ''; }})) && reads === 0;
  });
  const other = document.getElementById('child').contentWindow;
  check('constructor conversion TypeError uses callee realm', () => typeError(() => new other.StorageEvent('x', {key: Symbol()}), other));
  check('legacy conversion TypeError uses callee realm', () => typeError(() => other.StorageEvent.prototype.initStorageEvent.call(new StorageEvent('x'), 'x', false, false, Symbol()), other));
  check('constructor storageArea TypeError uses callee realm', () => typeError(() => new other.StorageEvent('x', {storageArea: {}}), other));
  check('legacy rejects forged receiver before converting arguments', () => {
    let conversions = 0;
    const type = {toString() { conversions++; return 'x'; }};
    return typeError(() => StorageEvent.prototype.initStorageEvent.call(Object.create(StorageEvent.prototype), type)) && conversions === 0;
  });
  check('legacy dispatch guard follows conversion and preserves state', () => {
    const target = new EventTarget();
    const event = new StorageEvent('storage', {key: 'keep'});
    let conversions = 0;
    target.addEventListener('storage', current => {
      current.initStorageEvent('after', true, true, {toString() { conversions++; return strings[4]; }});
    });
    target.dispatchEvent(event);
    return conversions === 1 && event.type === 'storage' && event.key === 'keep' && !event.bubbles && !event.cancelable;
  });
  if (location.protocol !== 'data:') {
    for (const [name, storageArea] of [['local', localStorage], ['session', sessionStorage], ['child', other.localStorage]]) {
      check(`constructor accepts branded ${name} Storage`, () => new StorageEvent('x', {storageArea}).storageArea === storageArea);
      check(`legacy accepts branded ${name} Storage`, () => legacy(null, null, null, '', storageArea).storageArea === storageArea);
    }
    let traps = 0;
    const author = new Proxy(localStorage, {get() { traps++; throw new Error('trap'); }, getPrototypeOf() { traps++; throw new Error('trap'); }});
    const revoked = Proxy.revocable(localStorage, {}); revoked.revoke();
    for (const [name, storageArea] of [['author proxy', author], ['revoked proxy', revoked.proxy], ['inherited instance', Object.create(localStorage)]]) {
      check(`constructor rejects ${name} Storage without traps`, () => typeError(() => new StorageEvent('x', {storageArea})) && traps === 0);
      check(`legacy rejects ${name} Storage without traps`, () => typeError(() => legacy(null, null, null, '', storageArea)) && traps === 0);
    }
    check('cross realm constructor accepts genuine parent Storage', () => new other.StorageEvent('x', {storageArea: localStorage, key: strings[4]}).storageArea === localStorage);
  }
  globalThis.__storageEventResults = {total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  const failed = rows.filter(row => !row.passed);
  return failed.length ? JSON.stringify(failed) : true;
})()
