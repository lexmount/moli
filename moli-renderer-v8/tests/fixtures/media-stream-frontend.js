(() => {
  const checks = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = (name, action) => {
    try { action(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const throws = (C, action, expectedName) => {
    let error; try { action(); } catch (caught) { error = caught; }
    assert(error instanceof C && (!expectedName || error.name === expectedName), 'Expected '+(expectedName || C.name)+', got '+error);
  };
  const realms = [globalThis, document.querySelector('iframe').contentWindow];
  for (const [index, realm] of realms.entries()) {
    const prefix = index ? 'iframe ' : 'main ';
    const C = realm.MediaStream, P = C.prototype;
    for (const [name, parent, length] of [['MediaStream', 'EventTarget', 0], ['MediaStreamTrack', 'EventTarget', 0], ['MediaStreamTrackEvent', 'Event', 2]]) {
      check(prefix+name+' interface', () => {
        const D = realm[name];
        assert(typeof D === 'function' && D.name === name && D.length === length, 'interface object');
        assert(Object.getPrototypeOf(D) === realm[parent] && Object.getPrototypeOf(D.prototype) === realm[parent].prototype, 'inheritance');
        assert(D.prototype.constructor === D && Object.prototype.toString.call(D.prototype) === '[object '+name+']', 'constructor and tag');
        throws(realm.TypeError, () => D());
      });
    }
    for (const [name, length] of [['getAudioTracks',0], ['getVideoTracks',0], ['getTracks',0], ['getTrackById',1], ['addTrack',1], ['removeTrack',1], ['clone',0]]) {
      check(prefix+'MediaStream '+name+' descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(P, name);
        assert(d && d.enumerable && d.configurable && d.writable && d.value.name === name && d.value.length === length, 'method descriptor');
        throws(realm.TypeError, () => d.value.call({}));
      });
    }
    for (const name of ['id', 'active', 'onaddtrack', 'onremovetrack']) {
      check(prefix+'MediaStream '+name+' descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(P, name);
        assert(d && d.enumerable && d.configurable && d.get && !!d.set === name.startsWith('on'), 'attribute descriptor');
        throws(realm.TypeError, () => d.get.call({}));
      });
    }
    for (const value of [undefined, null, false, 1, '', {}, {length: 0}, new realm.Array(1)]) {
      check(prefix+'MediaStream rejects '+typeof value+' '+String(value), () => throws(realm.TypeError, () => new C(value)));
    }
    for (const [name, factory] of [['empty', () => new C()], ['array', () => new C([])], ['set', () => new C(new Set())], ['copy', () => new C(new MediaStream())]]) {
      check(prefix+'MediaStream '+name+' construction', () => {
        const stream = factory();
        assert(stream instanceof C && !stream.active && stream.getTracks().length === 0 && stream.getAudioTracks().length === 0 && stream.getVideoTracks().length === 0, 'empty stream');
        assert(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(stream.id), 'random UUIDv4 identity');
        assert(stream.id === stream.id && !Object.hasOwn(stream, 'id') && stream.onaddtrack === null && stream.onremovetrack === null, 'stable prototype attributes');
      });
    }
    check(prefix+'MediaStream snapshots and clones', () => {
      const stream = new C(), tracks = stream.getTracks(); tracks.push({});
      assert(stream.getTracks().length === 0 && stream.getTracks() !== stream.getTracks(), 'independent snapshot');
      const cloned = P.clone.call(new MediaStream());
      assert(cloned instanceof MediaStream && !cloned.active && cloned.id !== stream.id, 'receiver realm clone');
      assert(stream.getTracks() instanceof realm.Array, 'receiver realm sequence');
      const copy = new C(stream);
      assert(copy.id !== stream.id && copy !== stream, 'copy gets new identity');
    });
    check(prefix+'MediaStream DOMString conversion', () => {
      const stream = new C(), marker = {}, log = [];
      assert(stream.getTrackById({toString() {log.push('string'); return '\ud800'}}) === null && log.join() === 'string', 'lossless conversion');
      let caught;
      try {stream.getTrackById({toString() {throw marker}})} catch(error) {caught = error}
      assert(caught === marker, 'conversion preserves exceptions');
      throws(realm.TypeError, () => stream.getTrackById(Symbol()));
      throws(realm.TypeError, () => stream.getTrackById());
    });
    check(prefix+'MediaStream iterator conversion', () => {
      let reads = 0, nexts = 0;
      const empty = {get [Symbol.iterator]() {reads++; return function() {return {next() {nexts++; return {done:true}}}}}};
      assert(new C(empty).getTracks().length === 0 && reads === 1 && nexts === 1, 'single GetMethod');
      const marker = {}; let caught;
      try {new C({get [Symbol.iterator]() {throw marker}})} catch(error) {caught = error}
      assert(caught === marker, 'iterator getter exception');
      throws(realm.TypeError, () => new C({[Symbol.iterator]: 1}));
    });
    check(prefix+'MediaStream conversion before prototype lookup', () => {
      const log = [];
      const iterable = {get [Symbol.iterator]() {log.push('iterator'); return function() {return {next() {log.push('next'); return {done:true}}}}}};
      const target = new Proxy(function() {}, {get(object, key) {if (key === 'prototype') log.push('prototype'); return Reflect.get(object, key)}});
      const stream = Reflect.construct(C, [iterable], target);
      assert(log.join() === 'iterator,next,prototype', 'WebIDL allocation order '+log);
      assert(Object.getOwnPropertyDescriptor(P, 'active').get.call(stream) === false, 'subclass brand');
    });
    check(prefix+'MediaStream native stream overload ignores iterator', () => {
      const stream = new MediaStream();
      Object.defineProperty(stream, Symbol.iterator, {get() {throw Error('author iterator getter')}});
      assert(new C(stream).getTracks().length === 0, 'stream overload selected by identity');
    });
    check(prefix+'MediaStream illegal receivers precede conversion', () => {
      const stream = new C(), revoked = Proxy.revocable(stream, {}); revoked.revoke();
      let reads = 0, traps = 0;
      const proxy = new Proxy(stream, {get() {traps++; throw Error('trap')}, getPrototypeOf() {traps++; throw Error('trap')}});
      for (const receiver of [{}, Object.create(P), Object.create(stream), proxy, revoked.proxy]) {
        throws(realm.TypeError, () => P.getTrackById.call(receiver, {toString() {reads++; throw Error('conversion')}}));
        throws(realm.TypeError, () => P.getTracks.call(receiver));
        throws(realm.TypeError, () => Object.getOwnPropertyDescriptor(P,'active').get.call(receiver));
        throws(realm.TypeError, () => P.addTrack.call(receiver, null));
      }
      assert(reads === 0 && traps === 0, 'brand check invokes no author code');
    });
    check(prefix+'MediaStream handlers use EventTarget listener ordering', () => {
      const stream = new C(), log = [];
      stream.addEventListener('addtrack', () => log.push(1));
      stream.onaddtrack = function(event) {assert(this === stream && event.target === stream && !event.isTrusted, 'handler callback'); log.push(2)};
      stream.addEventListener('addtrack', () => log.push(3));
      stream.onaddtrack = () => log.push(4);
      stream.dispatchEvent(new realm.Event('addtrack'));
      assert(log.join() === '1,4,3', 'replacement preserves position');
      stream.onaddtrack = 1;
      assert(stream.onaddtrack === null, 'nonobject setter is null');
      stream.onaddtrack = {};
      assert(stream.onaddtrack !== null, 'noncallable object retained');
      stream.onaddtrack = null;
      log.length = 0; stream.dispatchEvent(new realm.Event('addtrack'));
      assert(log.join() === '1,3', 'removing handler removes its listener');
    });
    for (const name of ['kind','id','label','enabled','muted','readyState','onmute','onunmute','onended','clone','stop']) {
      check(prefix+'MediaStreamTrack '+name+' descriptor and brand', () => {
        const d = Object.getOwnPropertyDescriptor(realm.MediaStreamTrack.prototype, name);
        assert(d && d.enumerable && d.configurable, 'track own property');
        if (d.get) throws(realm.TypeError, () => d.get.call({}));
        else throws(realm.TypeError, () => d.value.call({}));
      });
    }
    check(prefix+'MediaStreamTrackEvent required interface payload', () => {
      const E = realm.MediaStreamTrackEvent;
      for (const args of [[], ['type'], ['type', null], ['type', undefined], ['type', {}], ['type',{track:null}], ['type',{track:undefined}], ['type',{track:{}}]]) {
        throws(realm.TypeError, () => new E(...args));
      }
      const d = Object.getOwnPropertyDescriptor(E.prototype, 'track');
      assert(d && d.get && !d.set && d.enumerable && d.configurable, 'readonly event track');
      throws(realm.TypeError, () => d.get.call({}));
    });
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
