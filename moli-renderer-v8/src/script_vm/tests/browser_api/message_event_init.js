(() => {
  const rows = [];
  const observed = {};
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
  const channel = new MessageChannel();
  const otherChannel = new other.MessageChannel();
  const legacy = (data, origin, lastEventId, source, ports) => {
    const event = new MessageEvent('before');
    event.initMessageEvent('after', true, false, data, origin, lastEventId, source, ports);
    return event;
  };
  const texts = ['', '\ud800', '\udc00', '\ud83d\ude00', 'a\ud800b\udc00\u0000\ud83d\ude00'];
  for (let i = 0; i < texts.length; i++) {
    const text = texts[i];
    check(`constructor lastEventId UTF-16 ${i}`, () => new MessageEvent('x', {lastEventId: text}).lastEventId === text);
    check(`legacy lastEventId UTF-16 ${i}`, () => legacy(null, '', text).lastEventId === text);
  }
  for (const [value, expected] of [[undefined, ''], [null, 'null'], [23, '23']]) {
    check(`constructor lastEventId ${String(value)}`, () => new MessageEvent('x', {lastEventId: value}).lastEventId === expected);
    check(`legacy lastEventId ${String(value)}`, () => legacy(null, '', value).lastEventId === expected);
  }
  const scalar = 'a\ufffdb\ufffd\u0000\ud83d\ude00';
  check('constructor origin USVString follows current HTML IDL', () => new MessageEvent('x', {origin: texts[4]}).origin === scalar);
  check('legacy origin USVString follows current HTML IDL', () => legacy(null, texts[4], '').origin === scalar);
  for (const [value, expected] of [[undefined, ''], [null, 'null']]) {
    check(`constructor origin ${String(value)}`, () => new MessageEvent('x', {origin: value}).origin === expected);
    check(`legacy origin ${String(value)}`, () => legacy(null, value, '').origin === expected);
  }
  for (const [name, value] of [['omitted', undefined], ['null', null], ['symbol', Symbol()], ['function', () => {}], ['object', {x: 1}], ['port', channel.port1]]) {
    const expected = value === undefined ? null : value;
    check(`constructor any data ${name}`, () => new MessageEvent('x', {data: value}).data === expected);
    check(`legacy any data ${name}`, () => legacy(value, '', '').data === expected);
  }
  check('constructor defaults', () => {
    const event = new MessageEvent('x');
    return event.data === null && event.origin === '' && event.lastEventId === '' && event.source === null && Array.isArray(event.ports) && Object.isFrozen(event.ports) && !event.ports.length;
  });
  check('legacy method descriptor', () => {
    const descriptor = Object.getOwnPropertyDescriptor(MessageEvent.prototype, 'initMessageEvent');
    return descriptor.value.name === 'initMessageEvent' && descriptor.value.length === 1 && descriptor.enumerable && descriptor.writable && descriptor.configurable;
  });
  check('legacy default values and frozen array replacement', () => {
    const event = new MessageEvent('before', {data: {}, source: window, ports: [channel.port1], composed: true});
    const oldPorts = event.ports;
    const result = event.initMessageEvent('after');
    return result === undefined && event.type === 'after' && !event.bubbles && !event.cancelable && event.composed && event.data === null && event.origin === '' && event.lastEventId === '' && event.source === null && event.ports !== oldPorts && Object.isFrozen(event.ports) && event.ports.length === 0;
  });
  check('legacy missing required type', () => typeError(() => new MessageEvent('x').initMessageEvent()));
  for (const value of [false, 1, '', Symbol(), 1n]) {
    check(`invalid constructor dictionary ${typeof value}`, () => typeError(() => new MessageEvent('x', value)));
  }
  for (const value of [undefined, null]) {
    check(`nullish constructor dictionary ${String(value)}`, () => new MessageEvent('x', value).data === null);
  }
  const order = ['bubbles', 'cancelable', 'composed', 'data', 'lastEventId', 'origin', 'ports', 'source'];
  check('dictionary inherited and own conversion order', () => {
    const seen = [];
    new MessageEvent('x', new Proxy({}, {get(_target, name) { seen.push(name); }}));
    observed.dictionaryReads = seen;
    // Browsers may expose additional experimental dictionary members after
    // these standard members. Record them without weakening their ordering.
    return seen.slice(0, order.length).join() === order.join();
  });
  for (const stop of order) {
    check(`dictionary getter exception stops at ${stop}`, () => {
      const sentinel = {};
      const seen = [];
      const init = new Proxy({}, {get(_target, name) { seen.push(name); if (name === stop) throw sentinel; }});
      return throws(() => new MessageEvent('x', init), sentinel) && seen.join() === order.slice(0, order.indexOf(stop) + 1).join();
    });
  }
  for (const field of ['origin', 'lastEventId']) {
    check(`constructor ${field} converts with string hint once`, () => {
      let count = 0;
      const value = {[Symbol.toPrimitive](hint) { if (hint !== 'string') throw new Error(hint); count++; return 'converted'; }};
      return new MessageEvent('x', {[field]: value})[field] === 'converted' && count === 1;
    });
    check(`constructor ${field} Symbol rejects`, () => typeError(() => new MessageEvent('x', {[field]: Symbol()})));
    check(`legacy ${field} Symbol rejects`, () => typeError(() => legacy(null, field === 'origin' ? Symbol() : '', field === 'lastEventId' ? Symbol() : '')));
    check(`constructor ${field} conversion exception stops`, () => {
      const sentinel = {};
      const seen = [];
      const init = new Proxy({}, {get(_target, name) { seen.push(name); if (name === field) return {toString() { throw sentinel; }}; }});
      return throws(() => new MessageEvent('x', init), sentinel) && seen.join() === order.slice(0, order.indexOf(field) + 1).join();
    });
  }
  const sources = [['window', window], ['child window', other], ['port', channel.port1], ['child port', otherChannel.port1], ['null', null], ['undefined', undefined]];
  for (const [name, source] of sources) {
    const expected = source === undefined ? null : source;
    check(`constructor accepts ${name} source`, () => new MessageEvent('x', {source}).source === expected);
    check(`legacy accepts ${name} source`, () => legacy(null, '', '', source).source === expected);
  }
  let traps = 0;
  const handler = {get() { traps++; throw new Error('get trap'); }, getPrototypeOf() { traps++; throw new Error('prototype trap'); }};
  const revoked = Proxy.revocable(channel.port1, {}); revoked.revoke();
  const invalidSources = [['ordinary', {}], ['number', 1], ['string', 'source'], ['fake Window', Object.create(Window.prototype)], ['inherited Window', Object.create(window)], ['Window proxy', new Proxy(window, handler)], ['port proxy', new Proxy(channel.port1, handler)], ['revoked port', revoked.proxy], ['fake port', Object.create(MessagePort.prototype)], ['inherited port', Object.create(channel.port1)]];
  for (const [name, source] of invalidSources) {
    check(`constructor rejects ${name} source without traps`, () => typeError(() => new MessageEvent('x', {source})) && traps === 0);
    check(`legacy rejects ${name} source without traps`, () => typeError(() => legacy(null, '', '', source)) && traps === 0);
  }
  for (const [name, ports] of [['array', [channel.port1, otherChannel.port1]], ['set', new Set([channel.port1, otherChannel.port1])], ['generator', { *[Symbol.iterator]() { yield channel.port1; yield otherChannel.port1; } }]]) {
    check(`constructor ${name} port sequence`, () => {
      const event = new MessageEvent('x', {ports});
      return event.ports.length === 2 && event.ports[0] === channel.port1 && event.ports[1] === otherChannel.port1 && Object.isFrozen(event.ports) && event.ports === event.ports;
    });
    check(`legacy ${name} port sequence`, () => {
      const event = legacy(null, '', '', null, ports);
      return event.ports.length === 2 && event.ports[0] === channel.port1 && event.ports[1] === otherChannel.port1 && Object.isFrozen(event.ports);
    });
  }
  for (const [name, ports] of [['null', null], ['number', 1], ['string', 'ports'], ['array-like', {0: channel.port1, length: 1}], ['fake port', [{}]], ['port proxy', [new Proxy(channel.port1, handler)]], ['revoked port', [revoked.proxy]], ['hole', Array(1)]]) {
    check(`constructor rejects ${name} ports`, () => typeError(() => new MessageEvent('x', {ports})) && traps === 0);
    check(`legacy rejects ${name} ports`, () => typeError(() => legacy(null, '', '', null, ports)) && traps === 0);
  }
  check('constructor copies rather than freezes input array', () => {
    const input = [channel.port1];
    const event = new MessageEvent('x', {ports: input});
    input.push(channel.port2);
    return !Object.isFrozen(input) && event.ports !== input && event.ports.length === 1 && event.ports[0] === channel.port1;
  });
  for (const mode of ['constructor', 'legacy']) {
    check(`${mode} sequence reads iterator and next once`, () => {
      const seen = [];
      let index = 0;
      const ports = {get [Symbol.iterator]() { seen.push('iterator'); return function() { seen.push('call'); return {get next() { seen.push('next'); return () => ({done: index++ > 0, value: channel.port1}); }}; }; }};
      const event = mode === 'constructor' ? new MessageEvent('x', {ports}) : legacy(null, '', '', null, ports);
      return seen.join() === 'iterator,call,next' && event.ports.length === 1;
    });
    check(`${mode} sequence conversion failure propagates without IteratorClose`, () => {
      let closed = 0;
      let index = 0;
      const ports = {[Symbol.iterator]() { return {next() { return {done: false, value: index++ === 0 ? channel.port1 : {}}; }, return() { closed++; return {}; }}; }};
      const run = () => mode === 'constructor' ? new MessageEvent('x', {ports}) : legacy(null, '', '', null, ports);
      return typeError(run) && closed === 0;
    });
    check(`${mode} iterator exception identity`, () => {
      const sentinel = {};
      const ports = {[Symbol.iterator]() { return {next() { throw sentinel; }}; }};
      const run = () => mode === 'constructor' ? new MessageEvent('x', {ports}) : legacy(null, '', '', null, ports);
      return throws(run, sentinel);
    });
  }
  check('dictionary sequence converts before source getter', () => {
    const seen = [];
    const ports = {[Symbol.iterator]() { seen.push('iterator'); return [channel.port1][Symbol.iterator](); }};
    new MessageEvent('x', {get ports() { seen.push('ports'); return ports; }, get source() { seen.push('source'); return null; }});
    return seen.join() === 'ports,iterator,source';
  });
  check('legacy converts positional arguments before sequence', () => {
    const seen = [];
    const text = name => ({toString() { seen.push(name); return name; }});
    const ports = {[Symbol.iterator]() { seen.push('ports'); return [channel.port1][Symbol.iterator](); }};
    new MessageEvent('x').initMessageEvent(text('type'), false, false, null, text('origin'), text('lastEventId'), null, ports);
    return seen.join() === 'type,origin,lastEventId,ports';
  });
  check('legacy failed sequence conversion keeps all prior state', () => {
    const payload = {};
    const event = new MessageEvent('before', {bubbles: true, composed: true, data: payload, origin: 'old-origin', lastEventId: 'old-id', source: window, ports: [channel.port1]});
    const previous = event.ports;
    return typeError(() => event.initMessageEvent('after', false, true, null, '', '', null, [{}])) && event.type === 'before' && event.bubbles && !event.cancelable && event.composed && event.data === payload && event.origin === 'old-origin' && event.lastEventId === 'old-id' && event.source === window && event.ports === previous;
  });
  check('legacy dispatch guard follows argument conversion', () => {
    const event = new MessageEvent('message', {data: 'keep', ports: [channel.port1]});
    const ports = event.ports;
    const target = new EventTarget();
    let count = 0;
    target.addEventListener('message', current => current.initMessageEvent('after', true, true, null, '', {toString() { count++; return 'converted'; }}, null, []));
    target.dispatchEvent(event);
    return count === 1 && event.type === 'message' && event.data === 'keep' && event.ports === ports && !event.bubbles;
  });
  check('legacy borrowed initializer keeps FrozenArray event realm', () => {
    const event = new MessageEvent('x');
    other.MessageEvent.prototype.initMessageEvent.call(event, 'x', false, false, null, '', '', null, [otherChannel.port1]);
    observed.borrowedPortsUseEventRealm = Object.getPrototypeOf(event.ports) === Array.prototype;
    return observed.borrowedPortsUseEventRealm && event.ports[0] === otherChannel.port1;
  });
  check('constructor TypeError uses callee realm', () => typeError(() => new other.MessageEvent('x', {ports: [{}]}), other));
  check('legacy TypeError uses callee realm', () => typeError(() => other.MessageEvent.prototype.initMessageEvent.call(new MessageEvent('x'), 'x', false, false, null, '', '', {}, []), other));
  check('legacy forged receiver rejects before argument conversion', () => {
    let conversions = 0;
    const text = {toString() { conversions++; return 'x'; }};
    return typeError(() => MessageEvent.prototype.initMessageEvent.call(Object.create(MessageEvent.prototype), text)) && conversions === 0;
  });
  channel.port1.close(); channel.port2.close(); otherChannel.port1.close(); otherChannel.port2.close();
  globalThis.__messageEventResults = {rows, observed, total: rows.length, passed: rows.filter(row => row.passed).length};
  const failures = rows.filter(row => !row.passed);
  return failures.length ? JSON.stringify(failures) : true;
})()
