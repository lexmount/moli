(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const child = document.querySelector('iframe').contentWindow;
  const realms = [['top', window], ['child', child]];
  const interfaces = [
    ['MediaEncryptedEvent', 1, 'initData', 'initDataType'],
    ['MediaKeyMessageEvent', 2, 'message', 'messageType'],
  ];
  const tokens = ['license-request', 'license-renewal', 'license-release', 'individualization-request'];
  const typeError = (realm, callback) => {
    let error;
    try { callback(); } catch (caught) { error = caught; }
    return error !== undefined && Object.getPrototypeOf(error) === realm.TypeError.prototype;
  };
  for (const [calleeName, realm] of realms) {
    for (const [name, length, bufferMember, stringMember] of interfaces) {
      const Constructor = realm[name];
      const encrypted = name === 'MediaEncryptedEvent';
      const makeInit = (buffer, token = encrypted ? 'cenc' : tokens[0]) =>
        ({[bufferMember]: buffer, [stringMember]: token});
      const make = (buffer, ...args) => new Constructor('payload', {
        [bufferMember]: buffer,
        [stringMember]: args.length ? args[0] : encrypted ? 'cenc' : tokens[0],
      });
      const prefix = calleeName + '.' + name;
      check(prefix + ' constructor metadata', () => typeof Constructor === 'function' &&
        Constructor.name === name && Constructor.length === length &&
        Object.getPrototypeOf(Constructor.prototype) === realm.Event.prototype &&
        Constructor.prototype.constructor === Constructor);
      check(prefix + ' requires new', () => typeError(realm, () => Constructor('x', makeInit(new ArrayBuffer(1)))));
      check(prefix + ' requires arguments before type conversion', () => {
        let conversions = 0;
        const type = {toString() { conversions++; return 'x'; }};
        return typeError(realm, () => encrypted ? new Constructor() : new Constructor(type)) && conversions === 0;
      });
      for (const member of [bufferMember, stringMember]) {
        const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, member);
        check(prefix + '.' + member + ' readonly descriptor', () => descriptor !== undefined &&
          typeof descriptor.get === 'function' && descriptor.get.name === 'get ' + member &&
          descriptor.get.length === 0 && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
      }
      for (const [kind, dictionary] of [['omitted', undefined], ['null', null], ['empty', {}]]) {
        check(prefix + ' ' + kind + ' dictionary', () => {
          if (!encrypted) return typeError(realm, () => new Constructor('x', dictionary));
          const event = new Constructor('x', dictionary);
          return event.initData === null && event.initDataType === '' && event.type === 'x' &&
            event.bubbles === false && event.cancelable === false && event.composed === false && event.isTrusted === false;
        });
      }
      for (const dictionary of [0, 1, false, true, '', 'x', 1n, Symbol()]) {
        check(prefix + ' primitive dictionary ' + String(dictionary), () => typeError(realm, () => new Constructor('x', dictionary)));
      }
      check(prefix + ' EventInit then lexicographic payload conversion', () => {
        const order = [];
        const buffer = new ArrayBuffer(3);
        const dictionary = {
          get composed() { order.push('composed'); return true; },
          get cancelable() { order.push('cancelable'); return true; },
          get bubbles() { order.push('bubbles'); return true; },
          get [stringMember]() { order.push(stringMember); return {toString() { order.push('string'); return encrypted ? 'cenc' : tokens[0]; }}; },
          get [bufferMember]() { order.push(bufferMember); return buffer; },
          get ignored() { throw 42; },
        };
        const event = new Constructor({toString() { order.push('type'); return '\ud800'; }}, dictionary);
        return order.join(',') === ['type','bubbles','cancelable','composed',bufferMember,stringMember,'string'].join(',') &&
          event.type === '\ud800' && event[bufferMember] === buffer && event.bubbles && event.cancelable && event.composed;
      });
      for (const stop of ['type','bubbles','cancelable','composed',bufferMember,stringMember,'string']) {
        check(prefix + ' preserve exception at ' + stop, () => {
          const sentinel = {}; const order = [];
          const read = (key, value) => { order.push(key); if (key === stop) throw sentinel; return value; };
          const dictionary = {
            get bubbles() { return read('bubbles', true); },
            get cancelable() { return read('cancelable', true); },
            get composed() { return read('composed', true); },
            get [bufferMember]() { return read(bufferMember, new ArrayBuffer(1)); },
            get [stringMember]() { return read(stringMember, {toString() { return read('string', encrypted ? 'cenc' : tokens[0]); }}); },
          };
          let error;
          try { new Constructor({toString() { return read('type', 'x'); }}, dictionary); } catch (caught) { error = caught; }
          const keys = ['type','bubbles','cancelable','composed',bufferMember,stringMember,'string'];
          return error === sentinel && order.join(',') === keys.slice(0, keys.indexOf(stop) + 1).join(',');
        });
      }
      for (const [ownerName, owner] of realms) {
        const p = prefix + '/' + ownerName;
        const buffer = new owner.ArrayBuffer(3);
        new owner.Uint8Array(buffer).set([3,4,5]);
        check(p + ' retain ArrayBuffer identity and bytes', () => {
          const event = make(buffer);
          return event[bufferMember] === buffer && new Uint8Array(event[bufferMember]).join(',') === '3,4,5';
        });
        check(p + ' expose later writes to the same buffer', () => {
          const event = make(buffer); new owner.Uint8Array(buffer)[0] = 7;
          return event[bufferMember] === buffer && new Uint8Array(event[bufferMember])[0] === 7;
        });
        check(p + ' detach after construction keeps identity', () => {
          const payload = new owner.ArrayBuffer(2); const event = make(payload);
          owner.structuredClone(payload, {transfer: [payload]});
          return event[bufferMember] === payload && payload.byteLength === 0;
        });
        check(p + ' accepts already detached fixed ArrayBuffer', () => {
          const payload = new owner.ArrayBuffer(2); owner.structuredClone(payload, {transfer: [payload]});
          return make(payload)[bufferMember] === payload;
        });
        let traps = 0;
        const proxy = new owner.Proxy(buffer, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const revoked = owner.Proxy.revocable(buffer, {}); revoked.revoke();
        const invalid = [
          ['plain object', {}], ['prototype', owner.ArrayBuffer.prototype],
          ['forged', Object.create(owner.ArrayBuffer.prototype)], ['inherited', Object.create(buffer)],
          ['typed array', new owner.Uint8Array(buffer)], ['DataView', new owner.DataView(buffer)],
          ['proxy', proxy], ['revoked', revoked.proxy], ['number', 1], ['string', 'x'], ['symbol', Symbol()],
          ['resizable', new owner.ArrayBuffer(2, {maxByteLength: 4})],
        ];
        if (typeof owner.SharedArrayBuffer === 'function') invalid.push(['shared', new owner.SharedArrayBuffer(2)]);
        for (const [kind, payload] of invalid) {
          check(p + ' reject ' + kind + ' before string conversion', () => {
            let conversions = 0;
            return typeError(realm, () => make(payload, {toString() { conversions++; return encrypted ? 'cenc' : tokens[0]; }})) &&
              conversions === 0 && traps === 0;
          });
        }
        check(p + ' reject detached resizable ArrayBuffer', () => {
          const payload = new owner.ArrayBuffer(2, {maxByteLength: 4});
          owner.structuredClone(payload, {transfer: [payload]});
          return payload.resizable === true && typeError(realm, () => make(payload));
        });
        for (const payload of [null, undefined]) {
          check(p + ' nullable or required buffer ' + String(payload), () => encrypted
            ? make(payload).initData === null
            : typeError(realm, () => make(payload)));
        }
        if (encrypted) {
          for (const [input, output] of [['', ''], ['\ud800\udfff', '\ud800\udfff'], [null, 'null'], [undefined, ''], [42, '42'], [0n, '0'], [false, 'false']]) {
            check(p + ' DOMString ' + String(input), () => make(buffer, input).initDataType === output);
          }
          check(p + ' DOMString rejects Symbol', () => typeError(realm, () => make(buffer, Symbol())));
        } else {
          for (const token of tokens) check(p + ' enum ' + token, () => make(buffer, token).messageType === token);
          for (const token of ['', 'LICENSE-REQUEST', 'license-request ', 'unsupported', null, undefined, 0, true, Symbol()]) {
            check(p + ' reject enum ' + String(token), () => typeError(realm, () => make(buffer, token)));
          }
          check(p + ' missing required message before messageType read', () => {
            let reads = 0;
            return typeError(realm, () => new Constructor('x', {get messageType() { reads++; return tokens[0]; }})) && reads === 0;
          });
          check(p + ' missing required messageType', () => typeError(realm, () => new Constructor('x', {message: buffer})));
        }
        check(p + ' inherited dictionary members', () => {
          const event = new Constructor('x', Object.create({...makeInit(buffer), bubbles: true, composed: true}));
          return event[bufferMember] === buffer && event.bubbles && event.composed;
        });
        check(p + ' subclass and immutable prototype payloads', () => {
          class Custom extends Constructor {}
          const event = new Custom('x', makeInit(buffer));
          return Object.getPrototypeOf(event) === Custom.prototype && event instanceof Constructor &&
            event instanceof realm.Event && Object.prototype.toString.call(event) === '[object ' + name + ']' &&
            !Object.hasOwn(event, bufferMember) && !Object.hasOwn(event, stringMember) &&
            Reflect.set(event, bufferMember, null) === false && event[bufferMember] === buffer;
        });
        check(p + ' Event dispatch and preventDefault preserve payload', () => {
          const target = new owner.EventTarget(); const event = new Constructor('x', {...makeInit(buffer), cancelable: true});
          let delivered;
          target.addEventListener('x', e => { delivered = e; e.preventDefault(); });
          return target.dispatchEvent(event) === false && delivered === event && event.target === target &&
            event.defaultPrevented && !event.isTrusted && event[bufferMember] === buffer;
        });
        let receiver, receiverError;
        try { receiver = new owner[name]('x', makeInit(buffer)); } catch (error) { receiverError = error; }
        const author = new owner.Proxy(receiver ?? {}, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const revokedEvent = owner.Proxy.revocable(receiver ?? {}, {}); revokedEvent.revoke();
        let wrongEvent;
        try {
          wrongEvent = encrypted ? new owner.MediaKeyMessageEvent('x', {message: buffer, messageType: tokens[0]}) : new owner.MediaEncryptedEvent('x');
        } catch (_) { /* A missing shell constructor must not abort the baseline audit. */ }
        const receivers = [
          ['undefined', undefined], ['null', null], ['boolean', false], ['number', 1], ['string', 'x'],
          ['symbol', Symbol()], ['bigint', 1n], ['plain', {}], ['prototype', Constructor.prototype],
          ['forged', Object.create(Constructor.prototype)], ['inherited', Object.create(receiver ?? {})],
          ['proxy', author], ['revoked', revokedEvent.proxy], ['Event', new owner.Event('x')],
          ['wrong event', wrongEvent],
        ];
        for (const member of [bufferMember, stringMember]) {
          const getter = Object.getOwnPropertyDescriptor(Constructor.prototype, member)?.get;
          check(p + '.' + member + ' cross-realm genuine receiver', () => {
            if (receiverError) throw receiverError;
            return Reflect.apply(getter, receiver, []) === receiver[member];
          });
          for (const [kind, value] of receivers) {
            check(p + '.' + member + ' reject ' + kind + ' in callee realm', () => typeof getter === 'function' &&
              typeError(realm, () => Reflect.apply(getter, value, [])) && traps === 0);
          }
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(x => x.passed).length, checks};
  return true;
})()
