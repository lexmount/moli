(async () => {
  const checks = [];
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const check = async (name, body) => {
    try { await body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error), stack: error.stack}); }
  };
  const caught = body => { try { body(); } catch (error) { return error; } };
  const child = document.querySelector('iframe').contentWindow;
  for (const [realm, w] of [['main', window], ['iframe', child]]) {
    const run = (name, body) => check(`${realm}: ${name}`, async () => {
      const pc = new w.RTCPeerConnection();
      try { await body(pc); } finally { pc.close(); }
    });
    const audio = pc => pc.addTransceiver('audio').sender;
    const dtmf = pc => audio(pc).dtmf;
    const event = () => new w.RTCDTMFToneChangeEvent('tonechange', {tone: '1'});
    const unavailable = (value, tones = '') => {
      const error = caught(() => value.insertDTMF(tones));
      assert(error instanceof w.DOMException && error.name === 'InvalidStateError');
      assert(value.toneBuffer === '' && value.canInsertDTMF === false);
    };
    await run('DTMF interface is an illegal EventTarget constructor', () => {
      assert(w.RTCDTMFSender.name === 'RTCDTMFSender' && w.RTCDTMFSender.length === 0);
      assert(w.Object.getPrototypeOf(w.RTCDTMFSender.prototype) === w.EventTarget.prototype);
      assert(caught(() => new w.RTCDTMFSender()) instanceof w.TypeError);
      assert(caught(() => w.RTCDTMFSender()) instanceof w.TypeError);
    });
    await run('audio sender without a track has a stable native DTMF identity', pc => {
      const sender = audio(pc), value = sender.dtmf;
      assert(sender.track === null && value instanceof w.RTCDTMFSender && value instanceof w.EventTarget);
      assert(sender.dtmf === value && w.Object.getPrototypeOf(value) === w.RTCDTMFSender.prototype);
      assert(w.Object.prototype.toString.call(value) === '[object RTCDTMFSender]');
    });
    await run('video sender has null DTMF', pc => {
      assert(pc.addTransceiver('video').sender.dtmf === null);
    });
    await run('different audio senders have different DTMF objects', pc => {
      const first = dtmf(pc), second = dtmf(pc);
      assert(first instanceof w.RTCDTMFSender && second instanceof w.RTCDTMFSender && first !== second);
    });
    await run('sender DTMF descriptor is a readonly native accessor', pc => {
      const sender = audio(pc), d = w.Object.getOwnPropertyDescriptor(w.RTCRtpSender.prototype, 'dtmf');
      assert(typeof d.get === 'function' && d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable);
      assert(!w.Object.hasOwn(sender, 'dtmf') && !w.Reflect.set(sender, 'dtmf', null));
      assert(d.get.call(sender) === sender.dtmf);
    });
    await run('DTMF members have native WebIDL descriptors', pc => {
      const value = dtmf(pc), p = w.RTCDTMFSender.prototype;
      for (const name of ['toneBuffer', 'canInsertDTMF']) {
        const d = w.Object.getOwnPropertyDescriptor(p, name);
        assert(typeof d.get === 'function' && d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable);
        assert(!w.Object.hasOwn(value, name) && !w.Reflect.set(value, name, 123));
      }
      const h = w.Object.getOwnPropertyDescriptor(p, 'ontonechange');
      assert(typeof h.get === 'function' && typeof h.set === 'function' && h.get.length === 0 && h.set.length === 1 && h.enumerable && h.configurable);
      const m = w.Object.getOwnPropertyDescriptor(p, 'insertDTMF');
      assert(typeof m.value === 'function' && m.value.name === 'insertDTMF' && m.value.length === 1 && m.writable && m.enumerable && m.configurable);
    });
    await run('new sender is unavailable with an empty tone buffer and null handler', pc => {
      const value = dtmf(pc);
      assert(value.canInsertDTMF === false && value.toneBuffer === '' && value.ontonechange === null);
    });
    await run('nonobject handler assignments clear the handler', pc => {
      const value = dtmf(pc);
      for (const next of [null, undefined, false, true, 0, 'handler', Symbol('handler'), 1n]) {
        value.ontonechange = () => {}; value.ontonechange = next; assert(value.ontonechange === null);
      }
    });
    await run('handler callback object is retained without property access', pc => {
      const value = dtmf(pc); let reads = 0;
      const callback = new Proxy({}, {get() { reads++; throw Error('handler trap'); }});
      value.ontonechange = callback; assert(value.ontonechange === callback && reads === 0);
      value.ontonechange = null;
    });
    await run('handler participates in listener registration order', pc => {
      const value = dtmf(pc), trace = [];
      value.addEventListener('tonechange', () => trace.push('before'));
      value.ontonechange = function(e) { assert(this === value && e.currentTarget === value); trace.push('handler'); };
      value.addEventListener('tonechange', () => trace.push('after'));
      value.dispatchEvent(event()); assert(trace.join(',') === 'before,handler,after');
    });
    await run('handler replacement retains its position', pc => {
      const value = dtmf(pc), trace = [];
      value.ontonechange = () => trace.push('old'); value.addEventListener('tonechange', () => trace.push('listener'));
      value.ontonechange = () => trace.push('new'); value.dispatchEvent(event());
      assert(trace.join(',') === 'new,listener');
    });
    await run('cleared and reactivated handler is appended', pc => {
      const value = dtmf(pc), trace = [];
      value.ontonechange = () => trace.push('old'); value.addEventListener('tonechange', () => trace.push('listener'));
      value.ontonechange = null; value.ontonechange = () => trace.push('new'); value.dispatchEvent(event());
      assert(trace.join(',') === 'listener,new');
    });
    await run('once and removed listeners use EventTarget lifecycle', pc => {
      const value = dtmf(pc); let count = 0; const removed = () => count += 100;
      value.addEventListener('tonechange', () => count++, {once: true});
      value.addEventListener('tonechange', removed); value.removeEventListener('tonechange', removed);
      value.dispatchEvent(event()); value.dispatchEvent(event()); assert(count === 1);
    });
    await run('aborted listener is not invoked', pc => {
      const value = dtmf(pc), controller = new w.AbortController(); let count = 0;
      value.addEventListener('tonechange', () => count++, {signal: controller.signal}); controller.abort();
      value.dispatchEvent(event()); assert(count === 0);
    });
    await run('listener exception does not prevent subsequent dispatch', pc => {
      const value = dtmf(pc); let count = 0;
      value.ontonechange = () => { throw Error('expected DTMF callback failure'); };
      value.addEventListener('tonechange', () => count++); assert(value.dispatchEvent(event()) === true && count === 1);
    });
    await run('stopImmediatePropagation suppresses later handlers', pc => {
      const value = dtmf(pc); let count = 0;
      value.addEventListener('tonechange', e => e.stopImmediatePropagation()); value.ontonechange = () => count++;
      value.dispatchEvent(event()); assert(count === 0);
    });
    await run('manual tone event retains UTF16 payload and does not mutate the buffer', pc => {
      const value = dtmf(pc), e = new w.RTCDTMFToneChangeEvent('tonechange', {tone: 'a\ud800'});
      let observed; value.ontonechange = next => { observed = next; assert(next.eventPhase === 2); };
      assert(value.dispatchEvent(e) && observed === e && e.tone === 'a\ud800' && !e.isTrusted);
      assert(e.target === value && e.currentTarget === null && e.eventPhase === 0 && value.toneBuffer === '');
    });
    await run('insertDTMF requires its tones argument', pc => {
      const value = dtmf(pc); assert(caught(() => value.insertDTMF()) instanceof w.TypeError);
      assert(value.toneBuffer === '');
    });
    await run('unavailable state precedes validation of tone characters', pc => {
      const value = dtmf(pc);
      for (const tones of ['', '123Abcd#*,', 'invalid', '🎵', '\ud800', null, undefined]) unavailable(value, tones);
    });
    await run('all argument conversions precede unavailable state', pc => {
      const value = dtmf(pc), trace = [];
      const error = caught(() => value.insertDTMF({toString() { trace.push('tones'); return '1'; }},
        {valueOf() { trace.push('duration'); return 100; }}, {valueOf() { trace.push('gap'); return 70; }}));
      assert(error instanceof w.DOMException && error.name === 'InvalidStateError' && trace.join(',') === 'tones,duration,gap');
    });
    await run('each conversion exception is preserved without later conversions', pc => {
      const value = dtmf(pc), marker = {};
      for (let failing = 0; failing < 3; failing++) {
        const trace = [], convert = index => { trace.push(index); if (index === failing) throw marker; return index === 0 ? '1' : 100; };
        const error = caught(() => value.insertDTMF({toString() { return convert(0); }},
          {valueOf() { return convert(1); }}, {valueOf() { return convert(2); }}));
        assert(error === marker && trace.join(',') === [0, 1, 2].slice(0, failing + 1).join(','));
      }
    });
    await run('symbol tones and bigint numeric arguments fail conversion', pc => {
      const value = dtmf(pc);
      for (const args of [[Symbol('tone')], ['1', 1n], ['1', 100, 1n], ['1', Symbol('duration')], ['1', 100, Symbol('gap')]]) {
        assert(caught(() => value.insertDTMF(...args)) instanceof w.TypeError);
      }
    });
    await run('ordinary unsigned-long conversion accepts nonfinite and wrapping values', pc => {
      const value = dtmf(pc);
      for (const number of [NaN, Infinity, -Infinity, -1, 0, 0.5, 2 ** 32 + 100]) {
        const error = caught(() => value.insertDTMF('1', number, number));
        assert(error instanceof w.DOMException && error.name === 'InvalidStateError');
      }
    });
    await run('native brands reject forged author and revoked proxies before conversion or traps', pc => {
      const value = dtmf(pc); assert(value instanceof w.RTCDTMFSender); let conversions = 0, traps = 0;
      const proxy = new Proxy(value, {get() { traps++; throw Error('get trap'); }, getPrototypeOf() { traps++; throw Error('prototype trap'); }});
      const revoked = Proxy.revocable(value, {}); revoked.revoke();
      const p = w.RTCDTMFSender.prototype;
      for (const invalid of [{}, Object.create(value), Object.create(p), proxy, revoked.proxy]) {
        for (const name of ['toneBuffer', 'canInsertDTMF', 'ontonechange']) {
          assert(caught(() => w.Object.getOwnPropertyDescriptor(p, name).get.call(invalid)) instanceof w.TypeError);
        }
        assert(caught(() => p.insertDTMF.call(invalid, {toString() { conversions++; return '1'; }})) instanceof w.TypeError);
        assert(caught(() => w.Object.getOwnPropertyDescriptor(p, 'ontonechange').set.call(invalid, () => {})) instanceof w.TypeError);
      }
      assert(conversions === 0 && traps === 0);
    });
    await run('sender DTMF getter rejects forged and author proxies', pc => {
      const sender = audio(pc), getter = w.Object.getOwnPropertyDescriptor(w.RTCRtpSender.prototype, 'dtmf').get;
      let traps = 0; const proxy = new Proxy(sender, {getPrototypeOf() { traps++; throw Error('trap'); }});
      const revoked = Proxy.revocable(sender, {}); revoked.revoke();
      for (const invalid of [{}, Object.create(sender), Object.create(w.RTCRtpSender.prototype), proxy, revoked.proxy]) {
        assert(caught(() => getter.call(invalid)) instanceof w.TypeError);
      }
      assert(traps === 0);
    });
    await run('replaced global constructor does not change native DTMF allocation', pc => {
      const original = w.RTCDTMFSender;
      try { w.RTCDTMFSender = function() { throw Error('author constructor'); };
        const value = audio(pc).dtmf; assert(value instanceof original && w.Object.getPrototypeOf(value) === original.prototype);
      } finally { w.RTCDTMFSender = original; }
    });
    await run('replaceTrack preserves DTMF identity while track changes', async pc => {
      const sender = audio(pc), value = sender.dtmf; assert(value instanceof w.RTCDTMFSender);
      const track = pc.addTransceiver('audio').receiver.track;
      await sender.replaceTrack(track); assert(sender.track === track && sender.dtmf === value); unavailable(value);
      await sender.replaceTrack(null); assert(sender.track === null && sender.dtmf === value); unavailable(value);
    });
    await run('local offer and rollback do not enable tone transmission', async pc => {
      const sender = audio(pc), value = sender.dtmf; assert(value instanceof w.RTCDTMFSender);
      await pc.setLocalDescription(); unavailable(value, '12');
      await pc.setLocalDescription({type: 'rollback'}); unavailable(value, '12'); assert(sender.dtmf === value);
    });
    await run('stopping transceiver retains identity and remains unavailable', pc => {
      const transceiver = pc.addTransceiver('audio'), value = transceiver.sender.dtmf;
      transceiver.stop(); assert(transceiver.sender.dtmf === value); unavailable(value, '1');
    });
    await run('closing connection retains DTMF and EventTarget behavior', pc => {
      const sender = audio(pc), value = sender.dtmf; let count = 0;
      value.ontonechange = () => count++; pc.close(); assert(sender.dtmf === value); unavailable(value, '1');
      value.dispatchEvent(event()); assert(count === 1);
    });
    await run('shadowed public capability cannot authorize native sending', pc => {
      const value = dtmf(pc), getter = w.Object.getOwnPropertyDescriptor(w.RTCDTMFSender.prototype, 'canInsertDTMF').get;
      w.Object.defineProperty(value, 'canInsertDTMF', {value: true});
      assert(value.canInsertDTMF && getter.call(value) === false);
      const error = caught(() => value.insertDTMF('1')); assert(error instanceof w.DOMException && error.name === 'InvalidStateError');
      assert(value.toneBuffer === '');
    });
  }
  for (const [name, owner, callee] of [['main owner', window, child], ['iframe owner', child, window]]) {
    await check(`cross realm: ${name} keeps DTMF identity and callee errors`, () => {
      const pc = new owner.RTCPeerConnection();
      try {
        const sender = pc.addTransceiver('audio').sender;
        const getter = callee.Object.getOwnPropertyDescriptor(callee.RTCRtpSender.prototype, 'dtmf').get;
        const value = getter.call(sender);
        assert(value === sender.dtmf && value instanceof owner.RTCDTMFSender && !(value instanceof callee.RTCDTMFSender));
        const buffer = callee.Object.getOwnPropertyDescriptor(callee.RTCDTMFSender.prototype, 'toneBuffer').get;
        assert(buffer.call(value) === '');
        const error = caught(() => callee.RTCDTMFSender.prototype.insertDTMF.call(value, '1'));
        assert(error instanceof callee.DOMException && !(error instanceof owner.DOMException) && error.name === 'InvalidStateError');
        const invalid = caught(() => getter.call({})); assert(invalid instanceof callee.TypeError && !(invalid instanceof owner.TypeError));
      } finally { pc.close(); }
    });
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
