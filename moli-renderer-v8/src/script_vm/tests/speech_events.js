(() => {
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const throws = (run, Expected, label) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    assert(error instanceof Expected, label + ': ' + error);
    return error;
  };
  const errors = ['canceled', 'interrupted', 'audio-busy', 'audio-hardware', 'network',
    'synthesis-unavailable', 'synthesis-failed', 'language-unavailable',
    'voice-unavailable', 'text-too-long', 'invalid-argument', 'not-allowed'];
  const child = document.getElementById('child').contentWindow;
  for (const realm of [window, child]) {
    const utterance = new realm.SpeechSynthesisUtterance('hello');
    for (const name of ['SpeechSynthesisEvent', 'SpeechSynthesisErrorEvent']) {
      const Constructor = realm[name];
      const isError = name === 'SpeechSynthesisErrorEvent';
      const Parent = isError ? realm.SpeechSynthesisEvent : realm.Event;
      assert(typeof Constructor === 'function' && Constructor.name === name && Constructor.length === 2, name + ' constructor');
      assert(Object.getPrototypeOf(Constructor) === Parent, name + ' constructor inheritance');
      assert(Object.getPrototypeOf(Constructor.prototype) === Parent.prototype, name + ' prototype inheritance');
      assert(Constructor.prototype.constructor === Constructor, name + ' prototype constructor');
      const tag = Object.getOwnPropertyDescriptor(Constructor.prototype, Symbol.toStringTag);
      assert(tag.value === name && !tag.writable && !tag.enumerable && tag.configurable, name + ' tag');
      const callError = throws(() => Constructor(), realm.TypeError, name + ' requires new');
      if (realm !== window) assert(!(callError instanceof TypeError), name + ' callee error realm');
      for (const args of [[], ['x'], ['x', null], ['x', undefined], ['x', {}], ['x', { utterance: null }]]) {
        throws(() => Reflect.construct(Constructor, args), realm.TypeError, name + ' required utterance');
      }
      const init = isError ? { utterance, error: 'network' } : { utterance };
      const event = new Constructor('boundary', init);
      assert(event instanceof Constructor && event instanceof realm.SpeechSynthesisEvent && event instanceof realm.Event, name + ' identity');
      assert(Object.prototype.toString.call(event) === '[object ' + name + ']', name + ' instance tag');
      assert(event.type === 'boundary' && event.utterance === utterance, name + ' type and utterance');
      assert(event.charIndex === 0 && event.charLength === 0 && event.elapsedTime === 0 && event.name === '', name + ' payload defaults');
      assert(!event.bubbles && !event.cancelable && !event.composed && !event.isTrusted && event.target === null, name + ' Event defaults');
      if (isError) assert(event.error === 'network', 'error payload');
      const converted = new Constructor('end\uD800', {
        ...init, bubbles: true, cancelable: true, composed: true,
        charIndex: 2 ** 32 + 3.8, charLength: -1,
        elapsedTime: 1 / 3, name: 'word\uD800',
      });
      assert(converted.type === 'end\uD800' && converted.name === 'word\uD800', name + ' DOMString code units');
      assert(converted.charIndex === 3 && converted.charLength === 4294967295 && converted.elapsedTime === Math.fround(1 / 3), name + ' numeric conversion');
      assert(converted.bubbles && converted.cancelable && converted.composed, name + ' inherited EventInit');
      class Derived extends Constructor {}
      const derived = new Derived('boundary', init);
      assert(Object.getPrototypeOf(derived) === Derived.prototype && derived.utterance === utterance, name + ' new.target');
      const target = new realm.EventTarget();
      let delivered;
      target.addEventListener('boundary', value => { delivered = value; value.preventDefault(); });
      const cancelable = new Constructor('boundary', { ...init, cancelable: true });
      assert(target.dispatchEvent(cancelable) === false && delivered === cancelable && cancelable.defaultPrevented, name + ' native dispatch');
      for (const property of isError ? ['error'] : ['utterance', 'charIndex', 'charLength', 'elapsedTime', 'name']) {
        const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, property);
        assert(typeof descriptor.get === 'function' && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable, name + '.' + property + ' readonly accessor');
        assert(descriptor.get.call(event) === event[property], name + '.' + property + ' genuine receiver');
        for (const receiver of [{}, Object.create(Constructor.prototype), Object.create(event), new Proxy(event, {})]) {
          throws(() => descriptor.get.call(receiver), realm.TypeError, name + '.' + property + ' native brand');
        }
        const revoked = Proxy.revocable(event, {});
        revoked.revoke();
        throws(() => descriptor.get.call(revoked.proxy), realm.TypeError, name + '.' + property + ' revoked proxy');
      }
      for (const elapsedTime of [NaN, Infinity, -Infinity, 3.5e38, Symbol('time'), 1n]) {
        throws(() => new Constructor('x', { ...init, elapsedTime }), realm.TypeError, name + ' restricted float');
      }
      let trapReads = 0;
      const fake = new Proxy(utterance, { get() { trapReads++; throw Error('trap'); } });
      throws(() => new Constructor('x', { ...init, utterance: fake }), realm.TypeError, name + ' author proxy utterance');
      assert(trapReads === 0, name + ' brand check avoids author traps');
    }
    for (const error of errors) {
      assert(new realm.SpeechSynthesisErrorEvent('error', { utterance, error }).error === error, 'enum ' + error);
    }
    for (const error of [undefined, '', 'Network', 'unknown', null]) {
      throws(() => new realm.SpeechSynthesisErrorEvent('error', { utterance, error }), realm.TypeError, 'invalid error enum');
    }
    const reads = [];
    const ordered = { bubbles: false, cancelable: false, composed: false,
      charIndex: 0, charLength: 0, elapsedTime: 0, name: '', utterance, error: 'network' };
    const dictionary = new Proxy(ordered, { get(target, key) { reads.push(key); return target[key]; } });
    new realm.SpeechSynthesisErrorEvent({ toString() { reads.push('type'); return 'error'; } }, dictionary);
    assert(reads.join(',') === 'type,bubbles,cancelable,composed,charIndex,charLength,elapsedTime,name,utterance,error', 'inherited dictionary conversion order: ' + reads);
    const sentinel = {};
    let caught;
    try { new realm.SpeechSynthesisEvent('x', { utterance, get elapsedTime() { throw sentinel; } }); } catch (error) { caught = error; }
    assert(caught === sentinel, 'dictionary getter exception identity');
    caught = undefined;
    try { new realm.SpeechSynthesisEvent('x', { utterance, name: { toString() { throw sentinel; } } }); } catch (error) { caught = error; }
    assert(caught === sentinel, 'DOMString conversion exception identity');
    let errorReads = 0;
    throws(() => new realm.SpeechSynthesisErrorEvent('error', {
      utterance: {}, get error() { errorReads++; return 'network'; },
    }), realm.TypeError, 'base dictionary validation precedes derived members');
    assert(errorReads === 0, 'invalid utterance stops conversion');
  }
  const foreignUtterance = new child.SpeechSynthesisUtterance('foreign');
  const cross = new SpeechSynthesisEvent('end', { utterance: foreignUtterance });
  assert(cross.utterance === foreignUtterance, 'cross-realm native utterance accepted');
  const foreignGetter = Object.getOwnPropertyDescriptor(child.SpeechSynthesisEvent.prototype, 'charIndex').get;
  assert(foreignGetter.call(cross) === 0, 'cross-realm genuine receiver accepted');
  return 'ok';
})()
