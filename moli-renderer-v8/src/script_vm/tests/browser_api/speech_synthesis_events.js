(() => {
  const checks = [];
  const assert = (condition, message) => { if (!condition) throw new Error(message || 'assertion failed'); };
  const check = (name, run) => {
    try { run(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const throws = (run, prototype) => {
    let caught;
    try { run(); } catch (error) { caught = error; }
    assert(caught && Object.getPrototypeOf(caught) === prototype, 'exception must use callee TypeError');
  };
  const child = document.querySelector('iframe').contentWindow;
  const realms = [window, child];
  const errorTokens = ['canceled', 'interrupted', 'audio-busy', 'audio-hardware', 'network', 'synthesis-unavailable', 'synthesis-failed', 'language-unavailable', 'voice-unavailable', 'text-too-long', 'invalid-argument', 'not-allowed'];
  for (const [realmIndex, realm] of realms.entries()) {
    for (const kind of ['SpeechSynthesisEvent', 'SpeechSynthesisErrorEvent']) {
      const tag = `${realmIndex}:${kind}`;
      const Ctor = realm[kind];
      const isError = kind === 'SpeechSynthesisErrorEvent';
      const prototype = Ctor.prototype;
      const utterance = new realm.SpeechSynthesisUtterance('words');
      const init = (extras = {}, ...supplied) => ({utterance: supplied.length ? supplied[0] : utterance, ...(isError ? {error: 'not-allowed'} : {}), ...extras});
      const getter = member => {
        const owner = member === 'error' ? realm.SpeechSynthesisErrorEvent.prototype : realm.SpeechSynthesisEvent.prototype;
        const descriptor = Object.getOwnPropertyDescriptor(owner, member);
        assert(descriptor && typeof descriptor.get === 'function', 'native payload getter missing: ' + member);
        return descriptor.get;
      };
      check(tag + ':constructor metadata', () => {
        assert(typeof Ctor === 'function' && Ctor.name === kind && Ctor.length === 2);
        assert(Object.getPrototypeOf(prototype) === (isError ? realm.SpeechSynthesisEvent.prototype : realm.Event.prototype));
        assert(Object.getPrototypeOf(Ctor) === (isError ? realm.SpeechSynthesisEvent : realm.Event));
        assert(prototype.constructor === Ctor);
      });
      check(tag + ':without new', () => throws(() => Ctor('test', init()), realm.TypeError.prototype));
      check(tag + ':no arguments', () => throws(() => new Ctor(), realm.TypeError.prototype));
      check(tag + ':missing dictionary', () => throws(() => new Ctor('test'), realm.TypeError.prototype));
      check(tag + ':arity before type conversion', () => {
        let conversions = 0;
        throws(() => new Ctor({toString() { conversions++; return 'test'; }}), realm.TypeError.prototype);
        assert(conversions === 0);
      });
      for (const [index, dictionary] of [undefined, null, false, 7, 'str', Symbol('dict'), 1n, {}].entries()) {
        check(tag + ':invalid dictionary ' + index, () => throws(() => new Ctor('test', dictionary), realm.TypeError.prototype));
      }
      for (const [index, value] of [null, undefined, {}, Object.create(utterance), new realm.Event('test'), Object.create(realm.SpeechSynthesisUtterance.prototype)].entries()) {
        check(tag + ':invalid utterance ' + index, () => throws(() => new Ctor('test', init({}, value)), realm.TypeError.prototype));
      }
      let traps = 0;
      const author = new Proxy(utterance, {get() { traps++; throw 47; }, getPrototypeOf() { traps++; throw 48; }});
      const revoked = Proxy.revocable(utterance, {}); revoked.revoke();
      for (const [index, value] of [author, revoked.proxy].entries()) {
        check(tag + ':proxy utterance ' + index, () => {
          throws(() => new Ctor('test', init({}, value)), realm.TypeError.prototype);
          assert(traps === 0);
        });
      }
      for (const [ownerIndex, owner] of realms.entries()) {
        const source = new owner.SpeechSynthesisUtterance('source');
        check(tag + ':default payload owner ' + ownerIndex, () => {
          const event = new Ctor('test', init({}, source));
          assert(event.utterance === source && event.charIndex === 0 && event.charLength === 0 && Object.is(event.elapsedTime, 0) && event.name === '');
          assert(event.type === 'test' && event.bubbles === false && event.cancelable === false && event.composed === false && event.isTrusted === false);
          assert(Object.getPrototypeOf(event) === prototype && event instanceof realm.Event);
          assert(event instanceof realm.SpeechSynthesisEvent);
          if (isError) assert(event.error === 'not-allowed');
        });
        check(tag + ':payload identity owner ' + ownerIndex, () => {
          const event = new Ctor('type\ud800', init({bubbles: true, cancelable: true, composed: true, name: 'name\ud800\udfff', charIndex: -1, charLength: 4294967297, elapsedTime: 1 / 3}, source));
          assert(event.type === 'type\ud800' && event.name === 'name\ud800\udfff');
          assert(event.charIndex === 4294967295 && event.charLength === 1 && Object.is(event.elapsedTime, Math.fround(1 / 3)));
          assert(event.bubbles && event.cancelable && event.composed && !event.isTrusted);
          source.text = 'mutated'; assert(event.utterance === source && event.utterance.text === 'mutated');
        });
        check(tag + ':subclass owner ' + ownerIndex, () => {
          class Derived extends Ctor {}
          const event = new Derived('test', init({name: 'subclass'}, source));
          assert(Object.getPrototypeOf(event) === Derived.prototype && event instanceof Ctor && event.name === 'subclass' && event.utterance === source);
          assert(getter('utterance').call(event) === source);
        });
      }
      for (const member of ['utterance', 'charIndex', 'charLength', 'elapsedTime', 'name', ...(isError ? ['error'] : [])]) {
        check(tag + ':prototype descriptor ' + member, () => {
          const get = getter(member);
          const owner = member === 'error' ? realm.SpeechSynthesisErrorEvent.prototype : realm.SpeechSynthesisEvent.prototype;
          const descriptor = Object.getOwnPropertyDescriptor(owner, member);
          assert(descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
          assert(get.length === 0 && get.name === 'get ' + member && !Object.hasOwn(get, 'prototype'));
          const event = new Ctor('test', init());
          assert(!Object.hasOwn(event, member) && Reflect.set(event, member, 'replacement') === false);
        });
        const valid = (() => { try { return new Ctor('test', init()); } catch { return null; } })();
        const eventProxy = new Proxy(valid || {}, {get() { traps++; throw 49; }, getPrototypeOf() { traps++; throw 50; }});
        const eventRevoked = Proxy.revocable(valid || {}, {}); eventRevoked.revoke();
        for (const [receiverIndex, receiver] of [null, undefined, {}, prototype, new realm.Event('wrong'), utterance, Object.create(valid || prototype), eventProxy, eventRevoked.proxy].entries()) {
          check(tag + ':invalid receiver ' + member + ':' + receiverIndex, () => {
            const get = getter(member);
            throws(() => get.call(receiver), realm.TypeError.prototype);
            assert(traps === 0);
          });
        }
        check(tag + ':cross-realm getter ' + member, () => {
          const other = realms[1 - realmIndex];
          const event = new other[kind]('test', {utterance, name: 'foreign', ...(isError ? {error: 'network'} : {})});
          const expected = {utterance, charIndex: 0, charLength: 0, elapsedTime: 0, name: 'foreign', error: 'network'}[member];
          assert(Object.is(getter(member).call(event), expected));
        });
      }
      for (const member of ['charIndex', 'charLength']) {
        const values = [[undefined, 0], [null, 0], [true, 1], ['17.9', 17], [-1, 4294967295], [-1.9, 4294967295], [4294967297.8, 1], [NaN, 0], [Infinity, 0], [-Infinity, 0]];
        for (const [index, [value, expected]] of values.entries()) {
          check(tag + ':unsigned long ' + member + ':' + index, () => assert(new Ctor('test', init({[member]: value}))[member] === expected));
        }
        for (const [index, value] of [Symbol('number'), 1n].entries()) {
          check(tag + ':invalid number ' + member + ':' + index, () => throws(() => new Ctor('test', init({[member]: value})), realm.TypeError.prototype));
        }
      }
      const floats = [[undefined, 0], [null, 0], [-0, -0], [1 / 3, Math.fround(1 / 3)], [-1.25, -1.25], [Number.MIN_VALUE, 0], ['2.25', 2.25], [3.4028234663852886e38, Math.fround(3.4028234663852886e38)]];
      for (const [index, [value, expected]] of floats.entries()) {
        check(tag + ':finite float ' + index, () => assert(Object.is(new Ctor('test', init({elapsedTime: value})).elapsedTime, expected)));
      }
      for (const [index, value] of [NaN, Infinity, -Infinity, 3.5e38, -3.5e38, Symbol('float'), 1n].entries()) {
        check(tag + ':invalid float ' + index, () => throws(() => new Ctor('test', init({elapsedTime: value})), realm.TypeError.prototype));
      }
      for (const [index, [value, expected]] of [[undefined, ''], [null, 'null'], [false, 'false'], [1n, '1'], ['\ud800\udfff', '\ud800\udfff'], ['\ud800', '\ud800'], ['\udfff', '\udfff'], ['x\ud800a\udfff', 'x\ud800a\udfff']].entries()) {
        check(tag + ':DOMString name ' + index, () => assert(new Ctor('test', init({name: value})).name === expected));
      }
      check(tag + ':symbol name', () => throws(() => new Ctor('test', init({name: Symbol('name')})), realm.TypeError.prototype));
      check(tag + ':dictionary conversion order', () => {
        const order = [];
        const values = {bubbles: true, cancelable: true, composed: true, charIndex: {valueOf() { order.push('charIndex.valueOf'); return 7; }}, charLength: {valueOf() { order.push('charLength.valueOf'); return 8; }}, elapsedTime: {valueOf() { order.push('elapsedTime.valueOf'); return .5; }}, name: {toString() { order.push('name.toString'); return 'ordered'; }}, utterance, error: {toString() { order.push('error.toString'); return 'network'; }}};
        const dictionary = Object.create(null);
        const keys = ['bubbles', 'cancelable', 'composed', 'charIndex', 'charLength', 'elapsedTime', 'name', 'utterance', ...(isError ? ['error'] : [])];
        for (const key of [...keys].reverse()) Object.defineProperty(dictionary, key, {get() { order.push(key); return values[key]; }});
        const event = new Ctor({toString() { order.push('type.toString'); return 'ordered'; }}, dictionary);
        const expected = ['type.toString', 'bubbles', 'cancelable', 'composed', 'charIndex', 'charIndex.valueOf', 'charLength', 'charLength.valueOf', 'elapsedTime', 'elapsedTime.valueOf', 'name', 'name.toString', 'utterance', ...(isError ? ['error', 'error.toString'] : [])];
        assert(JSON.stringify(order) === JSON.stringify(expected), JSON.stringify(order));
        assert(event.charIndex === 7 && event.charLength === 8 && event.elapsedTime === .5 && event.name === 'ordered');
      });
      const keys = ['bubbles', 'cancelable', 'composed', 'charIndex', 'charLength', 'elapsedTime', 'name', 'utterance', ...(isError ? ['error'] : [])];
      for (const [stop, stopKey] of keys.entries()) {
        check(tag + ':getter exception boundary ' + stopKey, () => {
          const seen = [], marker = {stop};
          const dictionary = {};
          for (const key of keys) Object.defineProperty(dictionary, key, {get() { seen.push(key); if (key === stopKey) throw marker; return init({charIndex: 1, charLength: 2, elapsedTime: .5, name: 'x'})[key]; }});
          let caught; try {new Ctor('test', dictionary);} catch (error) {caught = error;}
          assert(caught === marker && JSON.stringify(seen) === JSON.stringify(keys.slice(0, stop + 1)));
        });
      }
      for (const member of ['charIndex', 'charLength', 'elapsedTime', 'name', ...(isError ? ['error'] : [])]) {
        check(tag + ':conversion exception ' + member, () => {
          const marker = {member};
          const value = {[Symbol.toPrimitive]() {throw marker;}};
          let caught; try {new Ctor('test', init({[member]: value}));} catch (error) {caught = error;}
          assert(caught === marker);
        });
      }
      check(tag + ':invalid utterance stops derived members', () => {
        let read = false;
        const dictionary = {utterance: {}, get error() {read = true; return 'network';}};
        throws(() => new Ctor('test', dictionary), realm.TypeError.prototype);
        assert(!read);
      });
      check(tag + ':inherited dictionary', () => {
        const event = new Ctor('test', Object.create(init({charIndex: 5, charLength: 6, name: 'inherited'})));
        assert(event.charIndex === 5 && event.charLength === 6 && event.name === 'inherited' && event.utterance === utterance);
      });
      check(tag + ':proxy dictionary', () => {
        const read = [];
        const dictionary = new Proxy(init(), {get(target, key) {read.push(key); return target[key];}});
        const event = new Ctor('test', dictionary);
        assert(event.utterance === utterance && JSON.stringify(read) === JSON.stringify(keys));
      });
      check(tag + ':callable dictionary', () => {
        function dictionary() {}
        for (const [key, value] of Object.entries(init({name: 'callable'}))) Object.defineProperty(dictionary, key, {value, configurable: true});
        assert(new Ctor('test', dictionary).name === 'callable');
      });
      check(tag + ':author global constructor replacement', () => {
        const saved = realm.SpeechSynthesisUtterance;
        try { realm.SpeechSynthesisUtterance = function Forged() {}; assert(new Ctor('test', init()).utterance === utterance); }
        finally { realm.SpeechSynthesisUtterance = saved; }
      });
      check(tag + ':dispatch preserves payload', () => {
        const event = new Ctor('speech-check', init({cancelable: true, charIndex: 3, name: 'dispatch'}));
        let calls = 0;
        utterance.addEventListener('speech-check', received => {
          calls++; assert(received === event && received.utterance === utterance && received.name === 'dispatch' && received.charIndex === 3); received.preventDefault();
        }, {once: true});
        assert(utterance.dispatchEvent(event) === false && calls === 1 && event.defaultPrevented);
      });
      if (isError) {
        for (const token of errorTokens) check(tag + ':error enum ' + token, () => assert(new Ctor('test', init({error: token})).error === token));
        for (const [index, token] of [undefined, null, '', 'cancelled', 'NETWORK', 'not-allowed ', Symbol('error'), 1n, 5].entries()) {
          check(tag + ':invalid error enum ' + index, () => throws(() => new Ctor('test', init({error: token})), realm.TypeError.prototype));
        }
        check(tag + ':error enum string conversion', () => assert(new Ctor('test', init({error: {toString() {return 'network';}}})).error === 'network'));
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
