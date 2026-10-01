(() => {
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: !!passed});
  const throws = (fn, Ctor = TypeError) => {
    try { fn(); return false; } catch (error) { return error instanceof Ctor; }
  };
  const throwsExactly = (fn, expected) => {
    try { fn(); return false; } catch (error) { return error === expected; }
  };
  const init = {errorDetail: 'sdp-syntax-error'};
  const numbers = ['receivedAlert', 'sctpCauseCode', 'sdpLineNumber', 'sentAlert'];
  const error = new RTCError(init, 'message');
  check('inheritance', error instanceof RTCError && error instanceof DOMException &&
    error instanceof Error && Object.getPrototypeOf(RTCError.prototype) === DOMException.prototype);
  check('DOMException fields', error.name === 'OperationError' && error.code === 0 &&
    error.message === 'message');
  check('constructor lengths', RTCError.length === 1 && RTCErrorEvent.length === 2);
  check('tags', Object.prototype.toString.call(error) === '[object RTCError]');
  check('new required', throws(() => RTCError(init)));
  for (const value of [undefined, null, {}, true, 1, 'dictionary', Symbol(), 1n,
    {errorDetail: undefined}, {errorDetail: null}, {errorDetail: 'invalid'}]) {
    check('invalid dictionary ' + String(value), throws(() => new RTCError(value)));
  }
  check('missing argument', throws(() => new RTCError()));
  for (const detail of ['data-channel-failure', 'dtls-failure', 'fingerprint-failure',
    'sctp-failure', 'sdp-syntax-error', 'hardware-encoder-not-available', 'hardware-encoder-error']) {
    check('enum ' + detail, new RTCError({errorDetail: detail}).errorDetail === detail);
  }
  const utf16 = '\ud800\0\udc00';
  for (const [message, expected] of [[undefined, ''], [null, 'null'], [false, 'false'],
    [2n, '2'], [utf16, utf16]]) {
    check('DOMString message ' + String(message), new RTCError(init, message).message === expected);
  }
  check('message default', new RTCError(init).message === '');
  check('Symbol message', throws(() => new RTCError(init, Symbol())));
  check('message conversion exception', throwsExactly(() => new RTCError(init, {
    toString() { throw error; }
  }), error));
  for (const name of numbers) {
    check(name + ' absent', error[name] === null);
    check(name + ' undefined', new RTCError({...init, [name]: undefined})[name] === null);
    check(name + ' null converts to zero', new RTCError({...init, [name]: null})[name] === 0);
    for (const value of [NaN, Infinity, -Infinity]) {
      check(name + ' nonfinite ' + value, new RTCError({...init, [name]: value})[name] === 0);
    }
    const signed = name === 'sctpCauseCode' || name === 'sdpLineNumber';
    for (const [value, expected] of [[-1, signed ? -1 : 4294967295],
      [4294967297, 1], [2147483648, signed ? -2147483648 : 2147483648], [3.9, 3]]) {
      check(name + ' numeric ' + value, new RTCError({...init, [name]: value})[name] === expected);
    }
    check(name + ' BigInt rejects', throws(() => new RTCError({...init, [name]: 1n})));
    check(name + ' Symbol rejects', throws(() => new RTCError({...init, [name]: Symbol()})));
  }
  const dictionaryReads = ['errorDetail', ...numbers];
  const reads = [];
  const dictionary = Object.create({...init, receivedAlert: 2, sctpCauseCode: 3,
    sdpLineNumber: 4, sentAlert: 5});
  const converted = new RTCError(new Proxy(dictionary, {
    get(object, name) { reads.push(name); return object[name]; }
  }), {toString() { reads.push('message'); return utf16; }});
  check('dictionary lexical order before message', reads.join() === [...dictionaryReads, 'message'].join());
  check('inherited dictionary members', converted.receivedAlert === 2 && converted.sctpCauseCode === 3 &&
    converted.sdpLineNumber === 4 && converted.sentAlert === 5 && converted.message === utf16);
  const sentinel = new RangeError('sentinel');
  for (const name of dictionaryReads) {
    const touched = [];
    const bad = new Proxy(dictionary, {get(object, key) {
      touched.push(key);
      if (key === name) throw sentinel;
      return object[key];
    }});
    check('getter failure stops at ' + name, throwsExactly(() => new RTCError(bad, {
      toString() { touched.push('message'); return ''; }
    }), sentinel) && touched.join() === dictionaryReads.slice(0, dictionaryReads.indexOf(name) + 1).join());
  }
  check('enum conversion exception', throwsExactly(() => new RTCError({errorDetail: {
    toString() { throw sentinel; }
  }}), sentinel));
  Object.defineProperty(Object.prototype, 'errorDetail', {
    configurable: true, get() { throw sentinel; }
  });
  try {
    check('null dictionary avoids prototype pollution', throws(() => new RTCError(null)));
    check('undefined dictionary avoids prototype pollution', throws(() => new RTCError(undefined)));
  } finally { delete Object.prototype.errorDetail; }
  let removedMemberRead = false;
  new RTCError({...init, get httpRequestStatusCode() { removedMemberRead = true; return 0; }});
  check('removed dictionary member ignored', !removedMemberRead);

  for (const name of ['errorDetail', ...numbers]) {
    const descriptor = Object.getOwnPropertyDescriptor(RTCError.prototype, name);
    check(name + ' descriptor', descriptor && descriptor.enumerable && descriptor.configurable &&
      descriptor.get.name === 'get ' + name && descriptor.get.length === 0 && !descriptor.set &&
      !Object.hasOwn(error, name));
    const original = error[name];
    error[name] = 'changed';
    check(name + ' readonly', error[name] === original && throws(() => {
      'use strict'; error[name] = 1;
    }));
    let traps = 0;
    const proxy = new Proxy(error, {get() { traps++; throw sentinel; },
      getPrototypeOf() { traps++; throw sentinel; }});
    const revoked = Proxy.revocable(error, {});
    revoked.revoke();
    for (const receiver of [null, undefined, {}, RTCError.prototype, Object.create(error),
      new DOMException(), proxy, revoked.proxy]) {
      check(name + ' invalid receiver', throws(() => descriptor.get.call(receiver)));
    }
    check(name + ' no author Proxy traps', traps === 0);
  }
  const event = new RTCErrorEvent(utf16, {error, bubbles: true, cancelable: true, composed: true});
  check('event inheritance and payload', event instanceof RTCErrorEvent && event instanceof Event &&
    event.error === error && event.error === event.error && event.type === utf16);
  check('event flags', event.bubbles && event.cancelable && event.composed && !event.isTrusted);
  check('event tag', Object.prototype.toString.call(event) === '[object RTCErrorEvent]');
  let typeConversions = 0;
  check('event arity before conversion', throws(() => new RTCErrorEvent({toString() {
    typeConversions++; return 'error';
  }})) && typeConversions === 0);
  check('event new required', throws(() => RTCErrorEvent('error', {error})));
  for (const dictionary of [undefined, null, {}, true, 1, 'dictionary', Symbol(), 1n,
    {error: null}, {error: undefined}, {error: {}}, {error: new DOMException()},
    {error: Object.create(error)}, {error: new Proxy(error, {})}]) {
    check('event invalid required member', throws(() => new RTCErrorEvent('error', dictionary)));
  }
  const eventReads = [];
  const eventInit = Object.create({bubbles: 1, cancelable: [], composed: 'yes', error});
  const convertedEvent = new RTCErrorEvent({toString() { eventReads.push('type'); return 'converted'; }},
    new Proxy(eventInit, {get(object, name) { eventReads.push(name); return object[name]; }}));
  check('inherited EventInit first', eventReads.join() === 'type,bubbles,cancelable,composed,error' &&
    convertedEvent.error === error && convertedEvent.bubbles && convertedEvent.cancelable && convertedEvent.composed);
  for (const name of ['bubbles', 'cancelable', 'composed', 'error']) {
    check('event dictionary exception ' + name, throwsExactly(() => new RTCErrorEvent('error', {
      ...eventInit, error, get [name]() { throw sentinel; }
    }), sentinel));
  }
  const descriptor = Object.getOwnPropertyDescriptor(RTCErrorEvent.prototype, 'error');
  check('event error descriptor', descriptor && descriptor.enumerable && descriptor.configurable &&
    descriptor.get.name === 'get error' && descriptor.get.length === 0 && !descriptor.set &&
    !Object.hasOwn(event, 'error'));
  for (const receiver of [{}, Event.prototype, new Event('error'), Object.create(event),
    new Proxy(event, {})]) {
    check('event getter invalid receiver', throws(() => descriptor.get.call(receiver)));
  }
  check('event error readonly', throws(() => { 'use strict'; event.error = null; }) && event.error === error);
  const target = new EventTarget();
  let delivered = false;
  target.addEventListener(utf16, received => {
    delivered = received === event && received.error === error && received.target === target &&
      received.currentTarget === target;
    received.preventDefault();
  });
  check('event dispatch', target.dispatchEvent(event) === false && delivered && event.defaultPrevented &&
    event.currentTarget === null);
  event.initEvent('again', false, false);
  check('event reinitialization retains error', event.error === error && event.type === 'again' &&
    !event.defaultPrevented && !event.bubbles && !event.cancelable);
  for (const value of [error, event]) {
    let cloneError;
    try { structuredClone(value); } catch (exception) { cloneError = exception; }
    check('nonserializable ' + value.constructor.name, cloneError instanceof DOMException &&
      cloneError.name === 'DataCloneError');
  }

  const child = document.querySelector('#child').contentWindow;
  const foreignError = new child.RTCError(init, utf16);
  check('foreign DOMException getter', Object.getOwnPropertyDescriptor(DOMException.prototype,
    'message').get.call(foreignError) === utf16);
  check('foreign required interface', new RTCErrorEvent('error', {error: foreignError}).error === foreignError);
  check('callee realm TypeError', throws(() => new child.RTCError({}), child.TypeError) &&
    throws(() => new child.RTCErrorEvent('error', {error: {}}), child.TypeError));
  check('foreign getter TypeError', throws(() => Object.getOwnPropertyDescriptor(child.RTCError.prototype,
    'errorDetail').get.call({}), child.TypeError));
  for (const [name, args, member, expected] of [
    ['RTCError', [init, utf16], 'message', utf16],
    ['RTCErrorEvent', ['error', {error: foreignError}], 'error', foreignError]
  ]) {
    const Ctor = window[name];
    class Derived extends Ctor {}
    const derived = new Derived(...args);
    check(name + ' subclass', derived instanceof Derived && derived instanceof Ctor && derived[member] === expected);
    const newTarget = child.Function('');
    newTarget.prototype = 0;
    let prototypeReads = 0;
    const fallback = Reflect.construct(Ctor, args, new Proxy(newTarget, {get(object, key) {
      if (key === 'prototype') prototypeReads++;
      return Reflect.get(object, key);
    }}));
    check(name + ' new target realm fallback', Object.getPrototypeOf(fallback) === child[name].prototype &&
      fallback[member] === expected && prototypeReads === 1);
  }
  Object.defineProperty(RTCError, Symbol.hasInstance, {
    configurable: true, value() { throw sentinel; }
  });
  try {
    check('interface validation avoids instanceof', new RTCErrorEvent('error', {error}).error === error &&
      throws(() => new RTCErrorEvent('error', {error: {}})));
  } finally { delete RTCError[Symbol.hasInstance]; }
  Object.setPrototypeOf(error, null);
  check('brand survives prototype removal', new RTCErrorEvent('error', {error}).error === error &&
    Object.getOwnPropertyDescriptor(RTCError.prototype, 'errorDetail').get.call(error) === init.errorDetail);
  globalThis.__rtcErrorResults = {rows, passed: rows.filter(row => row.passed).length, total: rows.length};
  return rows.every(row => row.passed) || JSON.stringify(rows.filter(row => !row.passed));
})()
