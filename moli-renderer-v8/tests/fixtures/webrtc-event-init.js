(() => {
  const checks = [];
  const assert = (value, name) => { if (!value) throw Error(name); };
  const check = (name, action) => {
    try { action(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const throws = (C, action) => {
    let error;
    try { action(); } catch (caught) { error = caught; }
    assert(error instanceof C, 'callee exception');
  };
  const frame = document.createElement('iframe');
  document.body.append(frame);
  const other = frame.contentWindow;
  for (const [realm, w] of [['main', window], ['iframe', other]]) {
    const C = w.RTCDTMFToneChangeEvent;
    for (const [name, properties, length] of [
      ['RTCDTMFToneChangeEvent', ['tone'], 1],
      ['RTCTrackEvent', ['receiver', 'track', 'streams', 'transceiver'], 2],
    ]) {
      const Constructor = w[name], P = Constructor.prototype;
      check(`${realm} ${name} constructor`, () => {
        assert(typeof Constructor === 'function' && Constructor.length === length && Constructor.name === name, 'constructor shape');
        assert(Object.getPrototypeOf(P) === w.Event.prototype && P.constructor === Constructor, 'event inheritance');
      });
      for (const property of properties) {
        const descriptor = Object.getOwnPropertyDescriptor(P, property);
        check(`${realm} ${name}.${property} descriptor`, () => {
          assert(descriptor && descriptor.enumerable && descriptor.configurable && typeof descriptor.get === 'function' && descriptor.set === undefined, 'readonly accessor');
        });
        for (const [index, receiver] of [null, undefined, 0, {}, Object.create(P), new w.Event('wrong')].entries()) {
          check(`${realm} ${name}.${property} receiver ${index}`, () => {
            assert(descriptor && typeof descriptor.get === 'function', 'getter must exist');
            throws(w.TypeError, () => descriptor.get.call(receiver));
          });
        }
      }
    }
    for (const [index, init] of [undefined, null, {}, [], () => {}].entries()) {
      check(`${realm} tone default dictionary ${index}`, () => {
        const event = new C('tone', init);
        assert(event instanceof C && event instanceof w.Event && event.tone === '' && event.type === 'tone', 'default payload');
        assert(!event.bubbles && !event.cancelable && !event.composed && !event.isTrusted && event.target === null, 'default header');
      });
    }
    const values = [[undefined, ''], [null, 'null'], [false, 'false'], [0, '0'], [19n, '19'],
      ['\ud800', '\ud800'], ['\udc00', '\udc00'], ['x\ud800\u0000y', 'x\ud800\u0000y'],
      ['\ud83e\udd95', '\ud83e\udd95'], [{toString() { return '1,2'; }}, '1,2']];
    for (const [index, [value, expected]] of values.entries()) {
      check(`${realm} tone DOMString ${index}`, () => {
        const event = new C('t\ud800', {tone: value, bubbles: true, cancelable: true, composed: true});
        assert(event.type === 't\ud800' && event.tone === expected && event.bubbles && event.cancelable && event.composed, 'lossless constructor strings');
        assert(!Object.hasOwn(event, 'tone') && Object.prototype.toString.call(event) === '[object RTCDTMFToneChangeEvent]', 'native prototype payload');
        event.initEvent('reset', false, false);
        assert(event.type === 'reset' && event.tone === expected && event.composed, 'legacy init preserves derived payload');
      });
    }
    check(`${realm} tone conversion and prototype ordering`, () => {
      const reads = [], prototype = Object.create(C.prototype);
      const target = new Proxy(function Target() {}, {get(object, key, receiver) {
        if (key === 'prototype') { reads.push('prototype'); return prototype; }
        return Reflect.get(object, key, receiver);
      }});
      const type = {toString() { reads.push('type'); return 'tone'; }};
      const init = new Proxy({bubbles: true, cancelable: true, composed: true, tone: {toString() { reads.push('string'); return '\ud800'; }}},
        {get(object, key) { reads.push(key); return object[key]; }});
      const event = Reflect.construct(C, [type, init], target);
      assert(reads.join() === 'type,bubbles,cancelable,composed,tone,string,prototype' && Object.getPrototypeOf(event) === prototype && event.tone === '\ud800', 'conversion before allocation');
    });
    check(`${realm} tone constructor arity before conversions`, () => {
      throws(w.TypeError, () => new C());
      let reads = 0;
      throws(w.TypeError, () => C({toString() { reads++; return 'x'; }}));
      throws(w.TypeError, () => new w.RTCTrackEvent({toString() { reads++; return 'x'; }}));
      assert(reads === 0, 'no argument conversion on invalid entry');
    });
    for (const [index, bad] of [1, 'x', true, Symbol(), 1n].entries()) {
      check(`${realm} tone dictionary invalid ${index}`, () => throws(w.TypeError, () => new C('x', bad)));
    }
    check(`${realm} tone Symbol conversion`, () => throws(w.TypeError, () => new C('x', {tone: Symbol()})));
    check(`${realm} tone getter exception identity`, () => {
      const marker = {}, reads = [];
      let error;
      try { new C('x', {get bubbles() { reads.push('bubbles'); throw marker; }, get tone() { reads.push('tone'); return ''; }}); }
      catch (caught) { error = caught; }
      assert(error === marker && reads.join() === 'bubbles', 'abrupt inheritance member');
      const target = new Proxy(function Target() {}, {get(object, key) { if (key === 'prototype') throw marker; return object[key]; }});
      try { Reflect.construct(C, ['x', {tone: {toString() { reads.push('tone'); return ''; }}}], target); }
      catch (caught) { error = caught; }
      assert(error === marker && reads.join() === 'bubbles,tone', 'newTarget exception after conversion');
    });
    check(`${realm} tone author and revoked Proxy receiver`, () => {
      const event = new C('x', {tone: '1'}), get = Object.getOwnPropertyDescriptor(C.prototype, 'tone').get;
      let traps = 0;
      const proxy = new Proxy(event, {get() { traps++; throw Error('trap'); }});
      const revoked = Proxy.revocable(event, {}); revoked.revoke();
      for (const bad of [proxy, revoked.proxy, Object.create(event)]) throws(w.TypeError, () => get.call(bad));
      assert(traps === 0, 'native receiver guard does not enter author Proxy');
      const cross = Object.getOwnPropertyDescriptor((w === window ? other : window).RTCDTMFToneChangeEvent.prototype, 'tone').get;
      assert(cross.call(event) === '1', 'genuine cross realm receiver');
    });
    check(`${realm} tone subclass and readonly payload`, () => {
      class Derived extends C {}
      const event = new Derived('x', {tone: 'A'});
      assert(event instanceof Derived && event instanceof C && event.tone === 'A', 'subclass allocation');
      assert(!Reflect.set(event, 'tone', 'B') && event.tone === 'A', 'readonly payload');
    });
    check(`${realm} tone manual EventTarget dispatch`, () => {
      const target = new w.EventTarget(), event = new C('x', {tone: '1', cancelable: true});
      let seen = 0;
      target.addEventListener('x', e => { assert(e === event && e.tone === '1', 'payload identity'); seen++; e.preventDefault(); });
      assert(target.dispatchEvent(event) === false && seen === 1 && event.defaultPrevented, 'event dispatch semantics');
    });
    for (const [index, init] of [undefined, null, {}, {receiver: null}, {receiver: {}}].entries()) {
      check(`${realm} track required native receiver ${index}`, () => throws(w.TypeError, () => new w.RTCTrackEvent('track', init)));
    }
    check(`${realm} track failed receiver stops other members`, () => {
      const reads = [];
      const init = new Proxy({bubbles: false, cancelable: false, composed: false, receiver: Object.create(w.RTCRtpReceiver.prototype)},
        {get(object, key) { reads.push(key); return object[key]; }});
      throws(w.TypeError, () => new w.RTCTrackEvent('track', init));
      assert(reads.join() === 'bubbles,cancelable,composed,receiver', 'required native interface conversion');
    });
    check(`${realm} track receiver property exception identity`, () => {
      const marker = {};
      let error;
      try { new w.RTCTrackEvent('track', {get receiver() { throw marker; }, get streams() { throw Error('late read'); }}); }
      catch (caught) { error = caught; }
      assert(error === marker, 'original property exception');
    });
  }
  frame.remove();
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
