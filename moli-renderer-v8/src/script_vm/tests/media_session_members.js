(async () => {
  const checks = [];
  const captureResults = [];
  const check = (name, run) => {
    try {
      const actual = run();
      checks.push({name, passed: actual === true, actual});
    } catch (error) {
      checks.push({name, passed: false, error: String(error)});
    }
  };
  const throws = (run, Constructor) => {
    try { run(); } catch (error) { return error instanceof Constructor; }
    return false;
  };
  const exactThrow = (run, value) => {
    try { run(); } catch (error) { return error === value; }
    return false;
  };
  const actions = ['play', 'pause', 'seekbackward', 'seekforward',
    'previoustrack', 'nexttrack', 'skipad', 'stop', 'seekto',
    'togglemicrophone', 'togglecamera', 'togglescreenshare', 'hangup',
    'previousslide', 'nextslide', 'enterpictureinpicture', 'voiceactivity'];
  const methods = [['setActionHandler', 2], ['setPositionState', 0],
    ['setMicrophoneActive', 1], ['setCameraActive', 1], ['setScreenshareActive', 1]];
  const realms = [['main', globalThis], ['child', document.querySelector('iframe').contentWindow]];
  for (const [label, realm] of realms) {
    const session = realm.navigator.mediaSession;
    const prototype = realm.MediaSession.prototype;
    const test = (name, run) => check(label + '/' + name, run);
    test('SameObject', () => session === realm.navigator.mediaSession);
    test('native brand', () => session instanceof realm.MediaSession);
    const playback = Object.getOwnPropertyDescriptor(prototype, 'playbackState');
    test('playback descriptor', () => !!playback && playback.enumerable && playback.configurable &&
      typeof playback.get === 'function' && typeof playback.set === 'function');
    test('initial playback', () => session.playbackState === 'none');
    for (const [method, length] of methods) {
      test(method + '/descriptor', () => {
        const value = Object.getOwnPropertyDescriptor(prototype, method);
        return !!value && value.enumerable && value.configurable && value.writable &&
          typeof value.value === 'function' && value.value.name === method && value.value.length === length;
      });
    }
    for (const state of ['paused', 'playing', 'none']) {
      test('playback/' + state, () => { session.playbackState = state; return session.playbackState === state; });
    }
    for (const [name, value] of [['invalid', 'invalid'], ['empty', ''], ['null', null], ['undefined', undefined]]) {
      test('playback/ignore ' + name, () => {
        session.playbackState = 'playing';
        session.playbackState = value;
        return session.playbackState === 'playing';
      });
    }
    test('playback/conversion', () => {
      let reads = 0;
      session.playbackState = {[Symbol.toPrimitive](hint) { reads++; return hint === 'string' ? 'paused' : ''; }};
      return reads === 1 && session.playbackState === 'paused';
    });
    const sentinel = {};
    test('playback/original exception', () => exactThrow(() => {
      session.playbackState = {toString() { throw sentinel; }};
    }, sentinel) && session.playbackState === 'paused');
    test('playback/Symbol', () => throws(() => { session.playbackState = Symbol(); }, realm.TypeError));
    for (const action of actions) {
      test('action/' + action, () => session.setActionHandler(action, null) === undefined);
    }
    test('action/function registration', () => {
      let calls = 0;
      session.setActionHandler('play', () => { calls++; });
      session.setActionHandler('play', undefined);
      return calls === 0;
    });
    test('action/callable Proxy', () => {
      let traps = 0;
      const callback = new Proxy(() => {}, {get() { traps++; }, apply() { traps++; }});
      session.setActionHandler('play', callback);
      session.setActionHandler('play', null);
      return traps === 0;
    });
    test('action/invalid enum', () => throws(() => session.setActionHandler('invalid', null), realm.TypeError));
    test('action/Symbol', () => throws(() => session.setActionHandler(Symbol(), null), realm.TypeError));
    test('action/non callable', () => throws(() => session.setActionHandler('play', {}), realm.TypeError));
    test('action/required arguments before conversion', () => {
      let reads = 0;
      const error = throws(() => session.setActionHandler({toString() { reads++; return 'play'; }}), realm.TypeError);
      return error && reads === 0;
    });
    test('action/enum before callback', () => {
      let traps = 0;
      const error = throws(() => session.setActionHandler('invalid', new Proxy({}, {get() { traps++; }})), realm.TypeError);
      return error && traps === 0;
    });
    test('action/original exception', () => exactThrow(() => {
      session.setActionHandler({toString() { throw sentinel; }}, null);
    }, sentinel));
    const positions = [
      ['missing', undefined, true], ['null', null, true], ['empty', {}, true],
      ['zero', {duration: 0}, true], ['duration only', {duration: 20}, true],
      ['forward', {duration: 20, playbackRate: 2, position: 3}, true],
      ['reverse', {duration: 20, playbackRate: -2, position: 3}, true],
      ['infinite duration', {duration: Infinity, position: 30}, true],
      ['coercion', {duration: '20', playbackRate: '2', position: '3'}, true],
      ['no duration', {position: 0}, false], ['negative duration', {duration: -1}, false],
      ['NaN duration', {duration: NaN}, false], ['negative position', {duration: 20, position: -1}, false],
      ['position over duration', {duration: 20, position: 21}, false],
      ['NaN position', {duration: 20, position: NaN}, false],
      ['infinite position', {duration: Infinity, position: Infinity}, false],
      ['zero rate', {duration: 20, playbackRate: 0}, false],
      ['negative zero rate', {duration: 20, playbackRate: -0}, false],
      ['NaN rate', {duration: 20, playbackRate: NaN}, false],
      ['infinite rate', {duration: 20, playbackRate: Infinity}, false],
    ];
    for (const [name, position, valid] of positions) {
      test('position/' + name, () => valid ? session.setPositionState(position) === undefined :
        throws(() => session.setPositionState(position), realm.TypeError));
    }
    test('position/dictionary read order', () => {
      const trace = [];
      session.setPositionState({
        get duration() { trace.push('duration'); return 20; },
        get playbackRate() { trace.push('playbackRate'); return 2; },
        get position() { trace.push('position'); return 3; },
      });
      return trace.join('|') === 'duration|playbackRate|position';
    });
    test('position/original exception and early exit', () => {
      const trace = [];
      const thrown = exactThrow(() => session.setPositionState({
        get duration() { trace.push('duration'); return 20; },
        get playbackRate() { trace.push('playbackRate'); throw sentinel; },
        get position() { trace.push('position'); return 3; },
      }), sentinel);
      return thrown && trace.join('|') === 'duration|playbackRate';
    });
    test('position/inherited dictionary', () => session.setPositionState(Object.create({duration: 20})) === undefined);
    const revoked = Proxy.revocable(session, {}); revoked.revoke();
    const receivers = [['ordinary', {}], ['forged', Object.create(prototype)],
      ['inherited', Object.create(session)], ['Proxy', new Proxy(session, {})], ['revoked', revoked.proxy]];
    for (const [name, receiver] of receivers) {
      test('receiver/' + name + '/playback', () => {
        let conversions = 0;
        const rejected = throws(() => playback.set.call(receiver, {toString() { conversions++; return 'playing'; }}), realm.TypeError);
        return rejected && conversions === 0;
      });
      test('receiver/' + name + '/action', () => {
        let conversions = 0;
        const rejected = throws(() => prototype.setActionHandler.call(receiver, {toString() { conversions++; return 'play'; }}, null), realm.TypeError);
        return rejected && conversions === 0;
      });
      test('receiver/' + name + '/position', () => {
        let reads = 0;
        const rejected = throws(() => prototype.setPositionState.call(receiver, {get duration() { reads++; return 20; }}), realm.TypeError);
        return rejected && reads === 0;
      });
      for (const method of ['setMicrophoneActive', 'setCameraActive', 'setScreenshareActive']) {
        let result, thrown;
        try { result = prototype[method].call(receiver, true); } catch (error) { thrown = error; }
        let reason;
        if (result && typeof result.then === 'function') await result.catch(error => { reason = error; });
        test('receiver/' + name + '/' + method, () => !thrown && result instanceof realm.Promise && reason instanceof realm.TypeError);
      }
    }
    for (const method of ['setMicrophoneActive', 'setCameraActive', 'setScreenshareActive']) {
      for (const active of [false, true]) {
        let result, thrown;
        try { result = session[method](active); } catch (error) { thrown = error; }
        let outcome;
        if (result && typeof result.then === 'function') {
          outcome = await result.then(value => ({value: String(value)}), error => ({error: error.name}));
        }
        captureResults.push({realm: label, method, active, outcome, thrown: thrown && String(thrown)});
        test(method + '/Promise/' + active, () => !thrown && result instanceof realm.Promise);
      }
      let result, thrown;
      try { result = session[method](); } catch (error) { thrown = error; }
      let reason;
      if (result && typeof result.then === 'function') await result.catch(error => { reason = error; });
      test(method + '/missing argument rejection', () => !thrown && result instanceof realm.Promise && reason instanceof realm.TypeError);
    }
  }
  const other = realms[1][1];
  check('cross realm/genuine receiver', () => {
    const setter = Object.getOwnPropertyDescriptor(MediaSession.prototype, 'playbackState').set;
    setter.call(other.navigator.mediaSession, 'playing');
    other.MediaSession.prototype.setActionHandler.call(navigator.mediaSession, 'play', null);
    return other.navigator.mediaSession.playbackState === 'playing';
  });
  check('cross realm/callee exception', () => {
    let error;
    try { other.MediaSession.prototype.setPositionState.call(navigator.mediaSession, {duration: -1}); }
    catch (caught) { error = caught; }
    return error instanceof other.TypeError && !(error instanceof TypeError);
  });
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks, captureResults};
  return true;
})()
