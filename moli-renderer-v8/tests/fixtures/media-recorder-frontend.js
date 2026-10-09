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
    return error;
  };
  for (const [index, realm] of [globalThis, document.querySelector('iframe').contentWindow].entries()) {
    const prefix = index ? 'iframe ' : 'main ';
    const C = realm.MediaRecorder, P = C.prototype;
    check(prefix+'MediaRecorder interface', () => {
      assert(C.name === 'MediaRecorder' && C.length === 1 && P.constructor === C, 'interface object');
      assert(Object.getPrototypeOf(C) === realm.EventTarget && Object.getPrototypeOf(P) === realm.EventTarget.prototype, 'interface inheritance');
      assert(Object.prototype.toString.call(P) === '[object MediaRecorder]', 'prototype tag');
      throws(realm.TypeError, () => C(new realm.MediaStream()));
    });
    for (const name of ['stream','mimeType','state','videoBitsPerSecond','audioBitsPerSecond','audioBitrateMode','onstart','onstop','ondataavailable','onpause','onresume','onerror']) {
      check(prefix+'MediaRecorder '+name+' descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(P, name);
        assert(d && d.enumerable && d.configurable && d.get && !!d.set === name.startsWith('on'), 'attribute descriptor');
        throws(realm.TypeError, () => d.get.call({}));
        if (d.set) throws(realm.TypeError, () => d.set.call({}, () => {}));
      });
    }
    for (const name of ['start','stop','pause','resume','requestData']) {
      check(prefix+'MediaRecorder '+name+' descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(P, name);
        assert(d && d.enumerable && d.configurable && d.writable && d.value.name === name && d.value.length === 0, 'method descriptor');
        throws(realm.TypeError, () => d.value.call({}));
      });
    }
    check(prefix+'MediaRecorder static descriptor and conversion', () => {
      const d = Object.getOwnPropertyDescriptor(C, 'isTypeSupported');
      assert(d && d.enumerable && d.configurable && d.writable && d.value.name === 'isTypeSupported' && d.value.length === 1, 'static method descriptor');
      assert(d.value.call({}, '') === true && !d.value.call(null, 'audio/banana'), 'static call needs no branded receiver');
      throws(realm.TypeError, () => d.value());
      throws(realm.TypeError, () => d.value(Symbol()));
      let calls = 0; assert(d.value({toString() {calls++; return ''}}) === true && calls === 1, 'single DOMString conversion');
      const marker = {}; let caught; try {d.value({toString() {throw marker}})} catch(error) {caught = error}
      assert(caught === marker, 'conversion exception identity');
      assert(!d.value('\ud800') && !d.value(undefined) && !d.value(null), 'nonempty strings unsupported');
    });
    for (const [name, arguments_] of [['missing',[]], ['undefined',[undefined]], ['null',[null]], ['ordinary',[{}]], ['forged',[Object.create(realm.MediaStream.prototype)]]]) {
      check(prefix+'MediaRecorder rejects '+name+' stream', () => throws(realm.TypeError, () => Reflect.construct(C, arguments_)));
    }
    for (const [name, options] of [['default',undefined], ['null',null], ['empty',{}], ['array',[]], ['callable',function() {}]]) {
      check(prefix+'MediaRecorder '+name+' options', () => {
        const stream = new MediaStream(), recorder = new C(stream, options);
        assert(recorder instanceof C && recorder.stream === stream && recorder.stream === recorder.stream && recorder.state === 'inactive' && recorder.mimeType === '', 'frontend state and SameObject stream');
        assert(Number.isInteger(recorder.audioBitsPerSecond) && recorder.audioBitsPerSecond >= 0 && Number.isInteger(recorder.videoBitsPerSecond) && recorder.videoBitsPerSecond >= 0, 'UA-selected unsigned targets');
        assert(['variable','constant'].includes(recorder.audioBitrateMode), 'resolved mode');
        for (const key of ['stream','mimeType','state','audioBitsPerSecond','videoBitsPerSecond']) assert(!Object.hasOwn(recorder,key), 'prototype attribute '+key);
      });
    }
    check(prefix+'MediaRecorder explicit bitrates convert unsigned values', () => {
      const recorder = new C(new realm.MediaStream(), {audioBitsPerSecond: 123.75, videoBitsPerSecond: 4294967299});
      assert(recorder.audioBitsPerSecond === 123 && recorder.videoBitsPerSecond === 3, 'unsigned-long conversion');
      const zero = new C(new realm.MediaStream(), {audioBitsPerSecond: NaN, videoBitsPerSecond: Infinity});
      assert(zero.audioBitsPerSecond === 0 && zero.videoBitsPerSecond === 0, 'nonfinite unsigned values convert to zero');
      const wrapped = new C(new realm.MediaStream(), {audioBitsPerSecond: -1});
      assert(wrapped.audioBitsPerSecond === 4294967295, 'normal unsigned conversion wraps');
      throws(realm.TypeError, () => new C(new realm.MediaStream(), {audioBitsPerSecond: 1n}));
    });
    check(prefix+'MediaRecorder total bitrate target and modes', () => {
      const recorder = new C(new realm.MediaStream(), {bitsPerSecond: 1000000, audioBitsPerSecond: 7, videoBitsPerSecond: 9, audioBitrateMode: 'constant'});
      assert(recorder.audioBitsPerSecond !== 7 && recorder.videoBitsPerSecond !== 9 && ['constant','variable'].includes(recorder.audioBitrateMode), 'total overrides individual targets and mode is selected by support');
      throws(realm.TypeError, () => new C(new realm.MediaStream(), {audioBitrateMode: 'quantizer'}));
      throws(realm.TypeError, () => new C(new realm.MediaStream(), {audioBitrateMode: null}));
    });
    check(prefix+'MediaRecorder option getters are lexicographic and precede support', () => {
      const log = [], options = {};
      const values = {audioBitrateMode:'variable', audioBitsPerSecond:1, bitsPerSecond:2, mimeType:'audio/banana', videoBitsPerSecond:3, videoKeyFrameIntervalCount:4, videoKeyFrameIntervalDuration:5};
      for (const key of Object.keys(values).reverse()) Object.defineProperty(options,key,{get() {log.push(key); return values[key]}});
      throws(realm.DOMException, () => new C(new realm.MediaStream(), options), 'NotSupportedError');
      assert(log.join() === Object.keys(values).join(), 'all declared dictionary members converted before format support check: '+log);
      const marker = {}, markerOptions = {mimeType:'audio/banana', get videoBitsPerSecond() {throw marker}};
      let caught; try {new C(new realm.MediaStream(), markerOptions)} catch(error) {caught=error}
      assert(caught === marker, 'later conversion error wins over unsupported MIME type');
    });
    check(prefix+'MediaRecorder inherited options and numeric conversion', () => {
      const log = [], options = Object.create({audioBitsPerSecond: 42});
      Object.defineProperty(options, 'videoBitsPerSecond', {get() {log.push('get'); return {valueOf() {log.push('number'); return 73}}}});
      const recorder = new C(new realm.MediaStream(), options);
      assert(recorder.audioBitsPerSecond === 42 && recorder.videoBitsPerSecond === 73 && log.join() === 'get,number', 'dictionary Get and ToNumber');
      for (const value of [NaN, Infinity, -Infinity]) throws(realm.TypeError, () => new C(new realm.MediaStream(), {videoKeyFrameIntervalDuration:value}));
    });
    check(prefix+'MediaRecorder format support agrees with construction', () => {
      for (const type of ['audio/banana', 'video/pineapple', '\ud800', 'video/webm', 'audio/ogg; codecs=opus']) {
        const supported = C.isTypeSupported(type);
        assert(typeof supported === 'boolean', 'support result');
        if (supported) assert(new C(new realm.MediaStream(), {mimeType:type}).mimeType === type, 'supported MIME snapshot');
        else throws(realm.DOMException, () => new C(new realm.MediaStream(), {mimeType:type}), 'NotSupportedError');
      }
      const log = [], recorder = new C(new realm.MediaStream(), {mimeType:{toString() {log.push('type'); return ''}}});
      assert(recorder.mimeType === '' && log.join() === 'type', 'MIME conversion');
      throws(realm.TypeError, () => new C(new realm.MediaStream(), {mimeType:Symbol()}));
    });
    check(prefix+'MediaRecorder inactive operations and start conversion', () => {
      const recorder = new C(new realm.MediaStream());
      assert(recorder.stop() === undefined && recorder.stop() === undefined && recorder.state === 'inactive', 'stop is idempotent');
      for (const name of ['pause','resume','requestData']) throws(realm.DOMException, () => recorder[name](), 'InvalidStateError');
      for (const timeslice of [undefined, null, 0, -1, NaN, Infinity]) throws(realm.DOMException, () => recorder.start(timeslice), 'NotSupportedError');
      throws(realm.TypeError, () => recorder.start(1n));
      let conversions = 0; throws(realm.DOMException, () => recorder.start({valueOf() {conversions++; return 1}}), 'NotSupportedError');
      assert(conversions === 1 && recorder.state === 'inactive' && recorder.mimeType === '', 'failed start leaves state and MIME untouched');
      const marker = {}; let caught; try {recorder.start({valueOf() {throw marker}})} catch(error) {caught=error}
      assert(caught === marker, 'start conversion error identity');
    });
    check(prefix+'MediaRecorder receiver checks precede conversion without Proxy traps', () => {
      const recorder = new C(new realm.MediaStream()), revoked = Proxy.revocable(recorder, {}); revoked.revoke();
      let conversions = 0, traps = 0;
      const proxy = new Proxy(recorder,{get() {traps++; throw Error('get')}, getPrototypeOf() {traps++; throw Error('prototype')}});
      for (const receiver of [{}, Object.create(P), Object.create(recorder), proxy, revoked.proxy]) {
        throws(realm.TypeError, () => P.start.call(receiver, {valueOf() {conversions++; throw Error('number')}}));
        for (const name of ['stop','pause','resume','requestData']) throws(realm.TypeError, () => P[name].call(receiver));
        for (const name of ['stream','state','mimeType','onstart']) throws(realm.TypeError, () => Object.getOwnPropertyDescriptor(P,name).get.call(receiver));
      }
      assert(conversions === 0 && traps === 0, 'brand validation runs first');
    });
    check(prefix+'MediaRecorder stream brand precedes option getters', () => {
      const stream = new realm.MediaStream(), revoked = Proxy.revocable(stream, {}); revoked.revoke();
      let reads = 0, traps = 0;
      const proxy = new Proxy(stream, {get() {traps++; throw Error('get')}, getPrototypeOf() {traps++; throw Error('prototype')}});
      for (const value of [{}, Object.create(stream), proxy, revoked.proxy]) throws(realm.TypeError, () => new C(value, {get mimeType() {reads++; return ''}}));
      assert(reads === 0 && traps === 0, 'stream conversion validates native identity');
    });
    for (const eventType of ['start','stop','dataavailable','pause','resume','error']) {
      check(prefix+'MediaRecorder on'+eventType+' listener ordering', () => {
        const recorder = new C(new realm.MediaStream()), log = [], slot = 'on'+eventType;
        assert(recorder[slot] === null, 'initial handler null');
        recorder.addEventListener(eventType, () => log.push(1));
        recorder[slot] = () => log.push(2);
        recorder.addEventListener(eventType, () => log.push(3));
        recorder[slot] = function(event) {assert(this === recorder && event.target === recorder && !event.isTrusted, 'synthetic event receiver'); log.push(4)};
        recorder.dispatchEvent(new realm.Event(eventType));
        assert(log.join() === '1,4,3', 'replacement preserves listener order');
        recorder[slot] = 7; assert(recorder[slot] === null, 'nonobject becomes null');
        recorder[slot] = {}; assert(recorder[slot] !== null, 'noncallable object retained');
        recorder[slot] = null; log.length = 0; recorder.dispatchEvent(new realm.Event(eventType));
        assert(log.join() === '1,3', 'handler deactivation');
      });
    }
    check(prefix+'MediaRecorder failed start and inactive stop emit no recording events', () => {
      const recorder = new C(new realm.MediaStream()), log = [];
      for (const type of ['start','stop','dataavailable','pause','resume','error']) recorder.addEventListener(type, () => log.push(type));
      throws(realm.DOMException, () => recorder.start(), 'NotSupportedError');
      recorder.stop(); assert(log.length === 0, 'no synthetic recording events');
    });
    check(prefix+'MediaRecorder conversion precedes newTarget prototype lookup', () => {
      const log = [], target = new Proxy(function() {}, {get(object,key) {if (key === 'prototype') log.push('prototype'); return Reflect.get(object,key)}});
      const options = {get mimeType() {log.push('mimeType'); return ''}};
      const recorder = Reflect.construct(C,[new realm.MediaStream(),options],target);
      assert(log.join() === 'mimeType,prototype', 'WebIDL constructor allocation order '+log);
      assert(Object.getOwnPropertyDescriptor(P,'state').get.call(recorder) === 'inactive', 'subclass native brand');
    });
    check(prefix+'MediaRecorder callee error realm with foreign genuine receivers', () => {
      const recorder = new MediaRecorder(new MediaStream());
      const error = throws(realm.DOMException, () => P.start.call(recorder), 'NotSupportedError');
      assert(Object.getPrototypeOf(error) === realm.DOMException.prototype, 'callee DOMException prototype');
      assert(Object.getOwnPropertyDescriptor(P,'state').get.call(recorder) === 'inactive', 'cross-realm native receiver');
    });
  }
  globalThis.__uiEventResults = {complete:true, total:checks.length, passed:checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
