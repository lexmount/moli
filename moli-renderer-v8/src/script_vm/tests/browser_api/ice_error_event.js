(() => {
  const Ctor = RTCPeerConnectionIceErrorEvent;
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: !!passed});
  const throws = (fn, Expected = TypeError) => {
    try { fn(); return false; } catch (error) { return error instanceof Expected; }
  };
  const throwsExactly = (fn, expected) => {
    try { fn(); return false; } catch (error) { return error === expected; }
  };
  const minimal = {errorCode: 701};
  const empty = new Ctor('error', minimal);
  const raw = '\ud800\0\udc00';
  const scalar = '\ufffd\0\ufffd';
  const payload = {address: raw, port: 4711, errorCode: 703, errorText: raw, url: raw};
  const event = new Ctor(raw, {...payload, bubbles: true, cancelable: true, composed: true});
  check('inheritance', event instanceof Ctor && event instanceof Event &&
    Object.getPrototypeOf(Ctor.prototype) === Event.prototype);
  check('tag', Object.prototype.toString.call(event) === '[object RTCPeerConnectionIceErrorEvent]');
  check('constructor length', Ctor.length === 2);
  check('default payload', empty.address === null && empty.port === null &&
    empty.url === '' && empty.errorText === '' && empty.errorCode === 701);
  check('EventInit defaults', !empty.bubbles && !empty.cancelable && !empty.composed && !empty.isTrusted);
  check('DOMString code units', event.type === raw && event.address === raw);
  check('USVString scalar values', event.url === scalar && event.errorText === scalar);
  check('explicit payload', event.port === 4711 && event.errorCode === 703);
  check('EventInit flags', event.bubbles && event.cancelable && event.composed && !event.isTrusted);
  check('new required', throws(() => Ctor('error', minimal)));
  check('missing type', throws(() => new Ctor()));
  let typeConversions = 0;
  check('arity before conversion', throws(() => new Ctor({toString() {
    typeConversions++; return 'type';
  }})) && typeConversions === 0);
  check('Symbol type', throws(() => new Ctor(Symbol(), minimal)));
  check('undefined type converts', new Ctor(undefined, minimal).type === 'undefined');
  check('null type converts', new Ctor(null, minimal).type === 'null');
  for (const init of [undefined, null, {}, true, 1, 'dictionary', Symbol(), 1n, {errorCode: undefined}]) {
    check('invalid dictionary', throws(() => new Ctor('error', init)));
  }
  check('null required number converts', new Ctor('error', {errorCode: null}).errorCode === 0);
  for (const [value, expected] of [[-1, 65535], [65537, 1], [65535, 65535], [3.9, 3],
    [-3.9, 65533], [NaN, 0], [Infinity, 0], [-Infinity, 0], [false, 0], ['65537', 1]]) {
    const converted = new Ctor('error', {errorCode: value, port: value});
    check('unsigned short ' + String(value), converted.errorCode === expected && converted.port === expected);
  }
  for (const name of ['port', 'errorCode']) {
    check(name + ' BigInt rejects', throws(() => new Ctor('error', {...minimal, [name]: 1n})));
    check(name + ' Symbol rejects', throws(() => new Ctor('error', {...minimal, [name]: Symbol()})));
  }
  for (const nullish of [null, undefined]) {
    const converted = new Ctor('error', {...minimal, address: nullish, port: nullish});
    check('nullable address and port', converted.address === null && converted.port === null);
  }
  check('nullable number object converts', new Ctor('error', {...minimal,
    port: {valueOf() { return null; }}
  }).port === 0);
  check('synthetic payload members are independent', new Ctor('error', {...minimal, port: 5}).port === 5);
  for (const name of ['address', 'url', 'errorText']) {
    check(name + ' Symbol rejects', throws(() => new Ctor('error', {...minimal, [name]: Symbol()})));
    for (const [value, expected] of [[false, 'false'], [2n, '2'], ['', ''], ['\ud83d\ude00', '\ud83d\ude00']]) {
      check(name + ' string conversion ' + String(value), new Ctor('error', {...minimal, [name]: value})[name] === expected);
    }
  }
  for (const name of ['url', 'errorText']) {
    check(name + ' null stringifies', new Ctor('error', {...minimal, [name]: null})[name] === 'null');
    check(name + ' undefined defaults', new Ctor('error', {...minimal, [name]: undefined})[name] === '');
  }
  const names = ['bubbles', 'cancelable', 'composed', 'address', 'errorCode', 'errorText', 'port', 'url'];
  const reads = [];
  const dictionary = Object.create({...payload, bubbles: 1, cancelable: [], composed: 'yes'});
  const converted = new Ctor({toString() { reads.push('type'); return 'converted'; }},
    new Proxy(dictionary, {get(object, name) { reads.push(name); return object[name]; }}));
  check('inherited dictionary order', reads.join() === ['type', ...names].join());
  check('inherited dictionary values', converted.address === raw && converted.url === scalar &&
    converted.errorText === scalar && converted.errorCode === 703 && converted.port === 4711 &&
    converted.bubbles && converted.cancelable && converted.composed);
  const sentinel = new RangeError('sentinel');
  for (const name of names) {
    const touched = [];
    const dictionary = new Proxy({...payload}, {get(object, key) {
      touched.push(key);
      if (key === name) throw sentinel;
      return object[key];
    }});
    check('getter failure stops at ' + name, throwsExactly(() => new Ctor('error', dictionary), sentinel) &&
      touched.join() === names.slice(0, names.indexOf(name) + 1).join());
  }
  check('type conversion exception', throwsExactly(() => new Ctor({toString() { throw sentinel; }}, minimal), sentinel));
  for (const name of ['address', 'errorCode', 'errorText', 'port', 'url']) {
    check(name + ' conversion exception', throwsExactly(() => new Ctor('error', {...minimal,
      [name]: {toString() { throw sentinel; }, valueOf() { throw sentinel; }}
    }), sentinel));
  }
  const touched = [];
  check('missing required member stops conversion', throws(() => new Ctor('error', new Proxy({}, {
    get(object, key) { touched.push(key); }
  }))) && touched.join() === 'bubbles,cancelable,composed,address,errorCode');
  Object.defineProperty(Object.prototype, 'bubbles', {configurable: true, get() { throw sentinel; }});
  try {
    check('null dictionary avoids prototype pollution', throws(() => new Ctor('error', null)));
    check('undefined dictionary avoids prototype pollution', throws(() => new Ctor('error', undefined)));
  } finally { delete Object.prototype.bubbles; }
  let hostCandidateRead = false;
  new Ctor('error', {...minimal, get hostCandidate() { hostCandidateRead = true; return 'legacy'; }});
  check('obsolete dictionary member ignored', !hostCandidateRead);
  check('obsolete attribute absent', !('hostCandidate' in Ctor.prototype));
  for (const [name, expected] of Object.entries({...payload, errorText: scalar, url: scalar})) {
    const descriptor = Object.getOwnPropertyDescriptor(Ctor.prototype, name);
    check(name + ' descriptor', descriptor && descriptor.enumerable && descriptor.configurable &&
      !descriptor.set && descriptor.get.name === 'get ' + name && descriptor.get.length === 0 && !Object.hasOwn(event, name));
    event[name] = 'changed';
    check(name + ' readonly', event[name] === expected && throws(() => { 'use strict'; event[name] = null; }));
    let traps = 0;
    const proxy = new Proxy(event, {get() { traps++; throw sentinel; }, getPrototypeOf() { traps++; throw sentinel; }});
    const revoked = Proxy.revocable(event, {});
    revoked.revoke();
    for (const receiver of [undefined, null, {}, Ctor.prototype, Event.prototype, new Event('error'),
      Object.create(event), proxy, revoked.proxy]) {
      check(name + ' invalid receiver', throws(() => descriptor.get.call(receiver)));
    }
    check(name + ' no author Proxy traps', traps === 0);
  }
  for (const name of ['type', 'bubbles', 'cancelable', 'composed']) {
    check(name + ' Event field readonly', throws(() => { 'use strict'; event[name] = 'changed'; }));
  }
  const target = new EventTarget();
  let delivered = false;
  target.addEventListener(raw, received => {
    delivered = received === event && received.address === raw && received.target === target &&
      received.currentTarget === target;
    received.preventDefault();
  });
  check('dispatch and cancellation', target.dispatchEvent(event) === false && delivered && event.defaultPrevented &&
    event.currentTarget === null && !event.isTrusted);
  event.initEvent('again', false, false);
  check('reinitialization', event.type === 'again' && !event.bubbles && !event.cancelable && !event.defaultPrevented);
  for (const [name, expected] of Object.entries({...payload, errorText: scalar, url: scalar})) {
    check(name + ' payload after initEvent', event[name] === expected);
  }
  const child = document.querySelector('#child').contentWindow;
  const foreign = new child.RTCPeerConnectionIceErrorEvent(raw, payload);
  for (const [name, expected] of Object.entries({...payload, errorText: scalar, url: scalar})) {
    check(name + ' cross realm getter', Object.getOwnPropertyDescriptor(Ctor.prototype, name).get.call(foreign) === expected);
    check(name + ' callee realm receiver error', throws(() => Object.getOwnPropertyDescriptor(
      child.RTCPeerConnectionIceErrorEvent.prototype, name).get.call({}), child.TypeError));
  }
  check('callee realm constructor error', throws(() => new child.RTCPeerConnectionIceErrorEvent('error', {}), child.TypeError));
  class Derived extends Ctor {}
  const derived = new Derived('derived', payload);
  check('subclass', derived instanceof Derived && derived instanceof Ctor && derived.port === payload.port);
  const newTarget = child.Function('');
  newTarget.prototype = 0;
  let prototypeReads = 0;
  const fallback = Reflect.construct(Ctor, ['error', payload], new Proxy(newTarget, {get(object, key) {
    if (key === 'prototype') prototypeReads++;
    return Reflect.get(object, key);
  }}));
  check('new target realm fallback', Object.getPrototypeOf(fallback) === child.RTCPeerConnectionIceErrorEvent.prototype &&
    fallback.address === raw && prototypeReads === 1);
  Object.setPrototypeOf(event, null);
  check('brand survives prototype removal', Object.getOwnPropertyDescriptor(Ctor.prototype, 'errorCode').get.call(event) === 703 &&
    Object.getOwnPropertyDescriptor(Event.prototype, 'type').get.call(event) === 'again');
  globalThis.__iceErrorResults = {rows, passed: rows.filter(row => row.passed).length, total: rows.length};
  return rows.every(row => row.passed) || JSON.stringify(rows.filter(row => !row.passed));
})()
