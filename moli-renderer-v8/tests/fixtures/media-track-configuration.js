(async () => {
  'use strict';
  const checks = [];
  const assert = (value, message = 'assertion failed') => { if (!value) throw Error(message); };
  function check(name, action) {
    try { action(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error), stack: error.stack}); }
  }
  const other = document.querySelector('iframe').contentWindow;
  for (const [realmName, w, borrowed] of [['main', globalThis, other], ['iframe', other, globalThis]]) {
    const prefix = realmName + ': ', proto = w.MediaStreamTrack.prototype;
    const hint = Object.getOwnPropertyDescriptor(proto, 'contentHint');
    check(prefix + 'native contentHint descriptor', () => {
      assert(hint && hint.enumerable && hint.configurable && hint.get.length === 0 && hint.set.length === 1);
      assert(String(hint.get).includes('[native code]') && String(hint.set).includes('[native code]'));
    });
    for (const name of ['getCapabilities', 'getConstraints', 'getSettings']) {
      check(prefix + name + ' native descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(proto, name);
        assert(d && d.enumerable && d.configurable && d.writable && typeof d.value === 'function' && d.value.length === 0);
        assert(String(d.value).includes('[native code]'));
      });
    }
    const pc = new w.RTCPeerConnection();
    try {
      for (const kind of ['audio', 'video']) {
        const track = pc.addTransceiver(kind).receiver.track;
        const name = prefix + kind + ': ';
        const valid = kind === 'audio' ? ['', 'speech', 'speech-recognition', 'music'] : ['', 'motion', 'detail', 'text'];
        const stable = kind === 'audio' ? 'speech' : 'detail';
        check(name + 'initial hint is unset', () => assert(track.contentHint === ''));
        for (const value of valid) {
          check(name + 'accepts hint ' + JSON.stringify(value), () => {
            track.contentHint = value; assert(track.contentHint === value);
          });
        }
        for (const [label, value] of [['other-kind', kind === 'audio' ? 'motion' : 'music'], ['case', 'SPEECH'],
            ['unknown', 'unknown'], ['surrogate', '\ud800'], ['undefined', undefined], ['null', null], ['number', 42]]) {
          check(name + 'ignores invalid hint ' + label, () => {
            track.contentHint = stable; track.contentHint = value; assert(track.contentHint === stable);
          });
        }
        check(name + 'omitted setter argument converts undefined', () => {
          const previous = track.contentHint;
          assert(hint.set.call(track) === undefined && track.contentHint === previous);
        });
        check(name + 'converts hint exactly once', () => {
          let conversions = 0;
          track.contentHint = {[Symbol.toPrimitive](type) {conversions++; assert(type === 'string'); return stable;}};
          assert(conversions === 1 && track.contentHint === stable);
        });
        check(name + 'propagates conversion exceptions and preserves state', () => {
          const marker = {}; let caught;
          track.contentHint = stable;
          try {track.contentHint = {toString() {throw marker;}};} catch (error) {caught = error;}
          assert(caught === marker && track.contentHint === stable);
          try {track.contentHint = Symbol();} catch (error) {caught = error;}
          assert(caught instanceof w.TypeError && track.contentHint === stable);
        });
        check(name + 'track clone copies hint independently', () => {
          track.contentHint = stable;
          const clone = track.clone();
          try {
            assert(clone !== track && clone.id !== track.id && clone.contentHint === stable);
            clone.contentHint = valid[3]; assert(clone.contentHint === valid[3] && track.contentHint === stable);
            track.contentHint = ''; assert(clone.contentHint === valid[3]);
          } finally {clone.stop();}
        });
        check(name + 'stream clone copies hint independently', () => {
          track.contentHint = stable;
          const cloned = new w.MediaStream([track]).clone().getTracks()[0];
          try {
            assert(cloned.contentHint === stable && cloned instanceof w.MediaStreamTrack);
            cloned.contentHint = valid[3]; assert(track.contentHint === stable);
          } finally {cloned.stop();}
        });
        for (const method of ['getCapabilities', 'getConstraints', 'getSettings']) {
          check(name + method + ' returns isolated dictionary snapshots', () => {
            const a = track[method](), b = track[method]();
            assert(a !== b && Object.getPrototypeOf(a) === w.Object.prototype && Object.getPrototypeOf(b) === w.Object.prototype);
            a.fixtureOnly = 1; a.width = 9999;
            assert(!('fixtureOnly' in track[method]()) && track[method]().width !== 9999);
            if (method === 'getConstraints') assert(Object.keys(b).length === 0);
          });
          check(name + method + ' borrowed binding returns callee realm dictionary', () => {
            const value = borrowed.MediaStreamTrack.prototype[method].call(track);
            assert(Object.getPrototypeOf(value) === borrowed.Object.prototype);
          });
        }
        check(name + 'source kind bypasses author properties', () => {
          Object.defineProperty(track, 'kind', {get() {throw Error('author kind getter');}, configurable: true});
          track.contentHint = stable; assert(track.contentHint === stable);
          delete track.kind;
        });
        check(name + 'disabled and stopped tracks retain mutable hints', () => {
          track.contentHint = stable; track.enabled = false; assert(track.contentHint === stable);
          track.stop(); assert(track.readyState === 'ended' && track.contentHint === stable);
          track.contentHint = valid[3]; assert(track.contentHint === valid[3]);
          const cloned = track.clone();
          assert(cloned.readyState === 'ended' && cloned.contentHint === valid[3]); cloned.stop();
        });
        check(name + 'rejects forged and author Proxy receivers before conversion', () => {
          let traps = 0, conversions = 0;
          const proxy = new w.Proxy(track, {get() {traps++; throw Error('get');}, getPrototypeOf() {traps++; throw Error('prototype');}});
          const revoked = w.Proxy.revocable(track, {}); revoked.revoke();
          const value = {toString() {conversions++; return stable;}};
          const getter = Object.getOwnPropertyDescriptor(borrowed.MediaStreamTrack.prototype, 'contentHint');
          for (const receiver of [{}, Object.create(track), Object.create(proto), proxy, revoked.proxy]) {
            for (const action of [() => getter.get.call(receiver), () => getter.set.call(receiver, value),
                ...['getCapabilities', 'getConstraints', 'getSettings'].map(method => () => borrowed.MediaStreamTrack.prototype[method].call(receiver))]) {
              let caught; try {action();} catch (error) {caught = error;}
              assert(caught instanceof borrowed.TypeError && !(caught instanceof w.TypeError));
            }
          }
          assert(traps === 0 && conversions === 0);
        });
      }
      check(prefix + 'intrinsic dictionary allocation ignores author Object constructor', () => {
        const track = pc.addTransceiver('video').receiver.track, saved = w.Object;
        try {
          w.Object = function() {throw Error('author Object constructor');};
          assert(saved.getPrototypeOf(track.getSettings()) === saved.prototype);
          assert(saved.getPrototypeOf(track.getConstraints()) === saved.prototype);
          assert(saved.getPrototypeOf(track.getCapabilities()) === saved.prototype);
        } finally {w.Object = saved; track.stop();}
      });
    } finally {pc.close();}
  }
  globalThis.__uiEventResults = {complete: true, checks, passed: checks.filter(row => row.passed).length, total: checks.length};
  return checks.every(row => row.passed);
})()
