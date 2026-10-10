(async () => {
  const checks = [];
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const caught = body => { try { body(); } catch (error) { return error; } };
  const rejected = async (w, body, constructor, name) => {
    const value = body();
    assert(value instanceof w.Promise, 'must return a callee realm Promise');
    let error; try { await value; } catch (reason) { error = reason; }
    assert(error instanceof constructor && (!name || error.name === name), 'wrong rejection: ' + error);
  };
  const check = async (name, body) => {
    try { await body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error), stack: error.stack}); }
  };
  const child = document.querySelector('iframe').contentWindow;
  for (const [realm, w] of [['main', window], ['iframe', child]]) {
    const run = (name, body) => check(`${realm}: ${name}`, async () => {
      const pc = new w.RTCPeerConnection();
      try { await body(pc); } finally { pc.close(); }
    });
    const peer = report => [...report.values()].find(value => value.type === 'peer-connection');
    const report = pc => pc.getStats();
    await run('three getStats operations have native WebIDL descriptors', () => {
      for (const name of ['RTCPeerConnection', 'RTCRtpSender', 'RTCRtpReceiver']) {
        const d = w.Object.getOwnPropertyDescriptor(w[name].prototype, 'getStats');
        assert(d && typeof d.value === 'function' && d.value.name === 'getStats' && d.value.length === 0);
        assert(d.writable && d.enumerable && d.configurable);
      }
    });
    await run('RTCStatsReport is an illegal constructor with Object inheritance', () => {
      assert(w.RTCStatsReport.name === 'RTCStatsReport' && w.RTCStatsReport.length === 0);
      assert(w.Object.getPrototypeOf(w.RTCStatsReport.prototype) === w.Object.prototype);
      assert(caught(() => new w.RTCStatsReport()) instanceof w.TypeError);
    });
    await run('omitted null and undefined selectors return native reports', async pc => {
      for (const call of [() => pc.getStats(), () => pc.getStats(null), () => pc.getStats(undefined)]) {
        const promise = call(); assert(promise instanceof w.Promise);
        const value = await promise;
        assert(value instanceof w.RTCStatsReport && !(value instanceof w.Map));
        assert(w.Object.getPrototypeOf(value) === w.RTCStatsReport.prototype);
        assert(w.Object.prototype.toString.call(value) === '[object RTCStatsReport]');
      }
    });
    await run('empty PC has one peer-connection dictionary with mandatory fields', async pc => {
      const value = await report(pc), stats = peer(value);
      assert(stats && value.get(stats.id).id === stats.id && value.has(stats.id));
      assert(typeof stats.id === 'string' && stats.id.length > 0 && Number.isFinite(stats.timestamp));
      assert(stats.dataChannelsOpened === 0 && stats.dataChannelsClosed === 0);
      for (const name of ['id', 'timestamp', 'type', 'dataChannelsOpened', 'dataChannelsClosed']) {
        const d = w.Object.getOwnPropertyDescriptor(stats, name);
        assert(d && d.writable && d.enumerable && d.configurable);
      }
      assert(w.Object.getPrototypeOf(stats) === w.Object.prototype);
    });
    await run('getStats settles after the current microtask checkpoint', async pc => {
      const trace = [], promise = pc.getStats().then(() => trace.push('stats'));
      await w.Promise.resolve(); assert(trace.length === 0);
      await promise; assert(trace.join(',') === 'stats');
    });
    await run('timestamps use native Performance time at gathering', async pc => {
      const t0 = w.performance.timeOrigin + w.performance.now();
      const value = peer(await report(pc));
      const t1 = w.performance.timeOrigin + w.performance.now();
      assert(value.timestamp + 1 >= t0 && value.timestamp <= t1 + 1);
    });
    await run('timestamps do not read author clock replacements', async pc => {
      const performance = w.performance, now = performance.now, origin = performance.timeOrigin;
      const originDesc = w.Object.getOwnPropertyDescriptor(performance, 'timeOrigin');
      const nowDesc = w.Object.getOwnPropertyDescriptor(performance, 'now');
      const dateNow = w.Date.now, t0 = origin + now.call(performance);
      try {
        w.Object.defineProperty(performance, 'timeOrigin', {value: -100, configurable: true});
        w.Object.defineProperty(performance, 'now', {value() { throw Error('author now'); }, configurable: true});
        w.Date.now = () => { throw Error('author Date.now'); };
        const value = peer(await report(pc)), t1 = origin + now.call(performance);
        assert(value.timestamp + 1 >= t0 && value.timestamp <= t1 + 1);
      } finally {
        w.Date.now = dateNow;
        if (originDesc) w.Object.defineProperty(performance, 'timeOrigin', originDesc); else delete performance.timeOrigin;
        if (nowDesc) w.Object.defineProperty(performance, 'now', nowDesc); else delete performance.now;
      }
    });
    await run('timestamp gathering never reads the replaceable window performance property', async pc => {
      const descriptor = w.Object.getOwnPropertyDescriptor(w, 'performance'); let reads = 0;
      try {
        w.Object.defineProperty(w, 'performance', {configurable: true, get() { reads++; throw Error('author performance getter'); }});
        assert(Number.isFinite(peer(await report(pc)).timestamp) && reads === 0);
      } finally {
        if (descriptor) w.Object.defineProperty(w, 'performance', descriptor); else delete w.performance;
      }
    });
    await run('IDs remain stable within their own peer connection', async pc => {
      const other = new w.RTCPeerConnection();
      try {
        const first = peer(await report(pc)), second = peer(await report(pc)), third = peer(await report(other));
        assert(first.id === second.id && typeof third.id === 'string' && second.timestamp >= first.timestamp);
      } finally { other.close(); }
    });
    await run('reports and dictionaries are fresh snapshots', async pc => {
      const first = await report(pc), value = peer(first), id = value.id;
      value.dataChannelsOpened = 123; value.type = 'author'; value.extra = true;
      const second = await report(pc), next = second.get(id);
      assert(second !== first && next !== value && next.type === 'peer-connection');
      assert(next.dataChannelsOpened === 0 && !w.Object.hasOwn(next, 'extra'));
      assert(value.extra && value.dataChannelsOpened === 123);
    });
    await run('closed PC still exposes peer-connection statistics with the same ID', async pc => {
      const id = peer(await report(pc)).id; pc.close();
      assert(peer(await report(pc)).id === id);
    });
    await run('closing before a queued stats result does not abort it', async pc => {
      const pending = report(pc); pc.close();
      assert(peer(await pending).dataChannelsClosed === 0);
    });
    await run('sender receiver and selected track have empty unnegotiated reports', async pc => {
      const t = pc.addTransceiver('audio');
      for (const call of [() => t.sender.getStats(), () => t.receiver.getStats(), () => pc.getStats(t.receiver.track)]) {
        const promise = call(); assert(promise instanceof w.Promise);
        const value = await promise; assert(value instanceof w.RTCStatsReport && value.size === 0);
      }
    });
    await run('stopped RTP endpoints retain callable stats operations', async pc => {
      const t = pc.addTransceiver('video'); t.stop();
      assert((await t.sender.getStats()).size === 0 && (await t.receiver.getStats()).size === 0);
      pc.close(); assert((await t.sender.getStats()).size === 0 && (await t.receiver.getStats()).size === 0);
    });
    await run('stopped and closed receiver tracks remain valid stats selectors', async pc => {
      const transceiver = pc.addTransceiver('audio'), track = transceiver.receiver.track;
      transceiver.stop(); assert((await pc.getStats(track)).size === 0);
      pc.close(); assert((await pc.getStats(track)).size === 0);
    });
    await run('a track on another PC rejects with InvalidAccessError', async pc => {
      const other = new w.RTCPeerConnection();
      try { await rejected(w, () => pc.getStats(other.addTransceiver('video').receiver.track), w.DOMException, 'InvalidAccessError'); }
      finally { other.close(); }
    });
    await run('cloned track identity cannot select the original receiver', async pc => {
      const original = pc.addTransceiver('audio').receiver.track, clone = original.clone();
      try { await rejected(w, () => pc.getStats(clone), w.DOMException, 'InvalidAccessError'); }
      finally { clone.stop(); }
    });
    await run('track matching both sender and receiver is ambiguous', async pc => {
      const track = pc.addTransceiver('audio').receiver.track; pc.addTransceiver(track);
      await rejected(w, () => pc.getStats(track), w.DOMException, 'InvalidAccessError');
    });
    await run('duplicate sender tracks are ambiguous', async pc => {
      const other = new w.RTCPeerConnection(), track = other.addTransceiver('audio').receiver.track;
      try {
        pc.addTransceiver(track); pc.addTransceiver(track);
        await rejected(w, () => pc.getStats(track), w.DOMException, 'InvalidAccessError');
      } finally { other.close(); }
    });
    await run('selector conversion rejects invalid values without coercing them', async pc => {
      let coercions = 0, traps = 0;
      const bad = {[Symbol.toPrimitive]() { coercions++; return null; }};
      const t = pc.addTransceiver('audio').receiver.track;
      const revoked = w.Proxy.revocable(t, {}); revoked.revoke();
      for (const value of [false, 0, 'audio', Symbol('x'), 1n, bad, w.Object.create(t), new w.Proxy(t, {get() { traps++; throw Error('trap'); }}), revoked.proxy])
        await rejected(w, () => pc.getStats(value), w.TypeError);
      assert(coercions === 0 && traps === 0);
    });
    await run('getStats brands reject forged and author proxy receivers', async pc => {
      const t = pc.addTransceiver('audio');
      for (const [prototype, real] of [[w.RTCPeerConnection.prototype, pc], [w.RTCRtpSender.prototype, t.sender], [w.RTCRtpReceiver.prototype, t.receiver]]) {
        let traps = 0;
        const revoked = w.Proxy.revocable(real, {}); revoked.revoke();
        for (const value of [{}, w.Object.create(prototype), w.Object.create(real), new w.Proxy(real, {get() { traps++; throw Error('trap'); }}), revoked.proxy])
          await rejected(w, () => prototype.getStats.call(value), w.TypeError);
        assert(traps === 0);
      }
    });
    await run('extra arguments to sender and receiver getStats are ignored', async pc => {
      const t = pc.addTransceiver('audio'), bomb = new w.Proxy({}, {get() { throw Error('extra argument'); }});
      assert((await t.sender.getStats(bomb, Symbol())).size === 0 && (await t.receiver.getStats(bomb)).size === 0);
    });
    await run('readonly maplike members have WebIDL descriptors and aliases', async pc => {
      const value = await report(pc), p = w.RTCStatsReport.prototype;
      for (const [name, length] of [['get', 1], ['has', 1], ['forEach', 1], ['entries', 0], ['keys', 0], ['values', 0]]) {
        const d = w.Object.getOwnPropertyDescriptor(p, name);
        assert(d && typeof d.value === 'function' && d.value.name === name && d.value.length === length);
        assert(d.writable && d.enumerable && d.configurable && !w.Object.hasOwn(value, name));
      }
      const size = w.Object.getOwnPropertyDescriptor(p, 'size');
      assert(size && size.get.length === 0 && size.set === undefined && size.enumerable && size.configurable);
      assert(p[Symbol.iterator] === p.entries && !w.Reflect.set(value, 'size', 99));
      for (const name of ['set', 'delete', 'clear']) assert(!w.Object.hasOwn(p, name) && value[name] === undefined);
    });
    await run('get and has convert DOMString keys once and preserve UTF16', async pc => {
      const value = await report(pc), id = peer(value).id; let conversions = 0;
      const key = {toString() { conversions++; return id; }};
      assert(value.get(key).id === id && conversions === 1);
      assert(value.has(key) && conversions === 2);
      for (const key of [undefined, null, true, 1, 1n, '', '\ud800', '\udfff'])
        assert(value.get(key) === undefined && value.has(key) === false);
      assert(value.get() === undefined && !value.has());
    });
    await run('maplike key conversion exceptions propagate unchanged', async pc => {
      const value = await report(pc), marker = {}, bad = {toString() { throw marker; }};
      for (const method of ['get', 'has']) {
        assert(caught(() => value[method](bad)) === marker);
        assert(caught(() => value[method](Symbol())) instanceof w.TypeError);
      }
    });
    await run('report iteration preserves value identity and map iterator prototypes', async pc => {
      const value = await report(pc), entries = [...value], keys = [...value.keys()], values = [...value.values()];
      assert(entries.length === value.size && keys.length === value.size && values.length === value.size);
      for (let i = 0; i < value.size; i++) assert(entries[i][0] === keys[i] && entries[i][1].id === values[i].id && value.get(keys[i]).id === values[i].id);
      const expected = w.Object.getPrototypeOf(new w.Map().entries());
      for (const iterator of [value.entries(), value.keys(), value.values(), value[Symbol.iterator]()]) {
        assert(w.Object.getPrototypeOf(iterator) === expected && iterator[Symbol.iterator]() === iterator);
        for (let i = 0; i < value.size; i++) assert(iterator.next().done === false);
        const first = iterator.next(), second = iterator.next();
        assert(first.done && second.done && first.value === undefined && second.value === undefined);
      }
    });
    await run('forEach uses value key owner and supplied thisArg', async pc => {
      const value = await report(pc), thisArg = {}, seen = [];
      const result = value.forEach(function(v, k, owner) { assert(this === thisArg && owner === value && owner.get(k).id === v.id); seen.push(k); return 1; }, thisArg);
      assert(result === undefined && seen.join() === [...value.keys()].join());
    });
    await run('forEach validates callback even on an empty report', async pc => {
      const value = await pc.addTransceiver('audio').sender.getStats();
      for (const callback of [undefined, null, false, 0, 'fn', {}, Symbol(), 1n])
        assert(caught(() => value.forEach(callback)) instanceof w.TypeError);
      assert(caught(() => value.forEach()) instanceof w.TypeError);
    });
    await run('forEach ignores callback.call and propagates callback errors', async pc => {
      const value = await report(pc), marker = {}; let count = 0;
      const callback = () => { count++; }; callback.call = () => { throw Error('author .call'); };
      value.forEach(callback); assert(count === value.size);
      assert(caught(() => value.forEach(() => { throw marker; })) === marker);
    });
    await run('forEach accepts callable proxies and propagates revoked calls', async pc => {
      const value = await report(pc); let count = 0;
      const callback = new w.Proxy(() => { throw Error('target must be trapped'); }, {apply(_, self, args) { assert(args[2] === value); count++; }});
      value.forEach(callback); assert(count === value.size);
      const revoked = w.Proxy.revocable(() => {}, {}); revoked.revoke();
      assert(caught(() => value.forEach(revoked.proxy)) instanceof w.TypeError);
    });
    await run('forEach is reentrant and supplies matching dictionary fields', async pc => {
      const value = await report(pc); let count = 0;
      value.forEach(v => { value.forEach(inner => { assert(inner.id === v.id && inner.type === v.type); count++; }); v.extra = 'visible'; assert(v.extra === 'visible'); });
      assert(count === value.size * value.size);
    });
    await run('maplike brand checks precede key conversion and proxy traps', async pc => {
      const value = await report(pc), p = w.RTCStatsReport.prototype; let conversions = 0, traps = 0;
      const key = {toString() { conversions++; throw Error('conversion'); }};
      const revoked = w.Proxy.revocable(value, {}); revoked.revoke();
      for (const receiver of [{}, w.Object.create(p), w.Object.create(value), new w.Proxy(value, {get() { traps++; throw Error('trap'); }}), revoked.proxy]) {
        for (const method of ['get', 'has']) assert(caught(() => p[method].call(receiver, key)) instanceof w.TypeError);
        for (const method of ['entries', 'keys', 'values', 'forEach']) assert(caught(() => p[method].call(receiver, () => {})) instanceof w.TypeError);
        assert(caught(() => w.Object.getOwnPropertyDescriptor(p, 'size').get.call(receiver)) instanceof w.TypeError);
      }
      assert(conversions === 0 && traps === 0);
    });
    await run('author constructor replacements do not affect native stats snapshots', async pc => {
      const names = ['RTCStatsReport', 'Map', 'Promise', 'Object'], saved = names.map(name => w[name]);
      try {
        for (const name of names) w[name] = function() { throw Error('author constructor: ' + name); };
        const promise = pc.getStats(), value = await promise;
        assert(promise instanceof saved[2] && value instanceof saved[0] && saved[3].getPrototypeOf(peer(value)) === saved[3].prototype);
      } finally { names.forEach((name, i) => { w[name] = saved[i]; }); }
    });
    await run('author Map prototype replacements do not affect native iteration', async pc => {
      const value = await report(pc), p = w.Map.prototype, names = ['get', 'has', 'entries', 'keys', 'values', 'forEach'];
      const saved = names.map(name => p[name]);
      try {
        for (const name of names) p[name] = () => { throw Error('author Map method: ' + name); };
        assert(value.get(peer(value).id).id === peer(value).id);
        assert([...value.entries()].length === value.size && [...value.keys()].length === value.size);
        let count = 0; value.forEach(() => count++); assert(count === value.size);
      } finally { names.forEach((name, i) => { p[name] = saved[i]; }); }
    });
  }
  await check('borrowed getStats creates Promise report and dictionaries in the callee realm', async () => {
    const pc = new child.RTCPeerConnection();
    try {
      const promise = RTCPeerConnection.prototype.getStats.call(pc), value = await promise;
      assert(promise instanceof Promise && !(promise instanceof child.Promise));
      assert(value instanceof RTCStatsReport && !(value instanceof child.RTCStatsReport));
      assert(Object.getPrototypeOf([...value.values()][0]) === Object.prototype);
      const reverse = child.RTCPeerConnection.prototype.getStats.call(pc), other = await reverse;
      assert(reverse instanceof child.Promise && other instanceof child.RTCStatsReport && Object.getPrototypeOf([...other.values()][0]) === child.Object.prototype);
    } finally { pc.close(); }
  });
  await check('borrowed maplike methods create callee realm iterators and preserve report values', async () => {
    const pc = new child.RTCPeerConnection();
    try {
      const value = await pc.getStats(), it = RTCStatsReport.prototype.entries.call(value), pair = it.next();
      assert(Object.getPrototypeOf(it) === Object.getPrototypeOf(new Map().entries()));
      assert(Object.getPrototypeOf(pair) === Object.prototype && Object.getPrototypeOf(pair.value) === Array.prototype);
      assert(pair.value[1].id === value.get(pair.value[0]).id && Object.getPrototypeOf(pair.value[1]) === child.Object.prototype);
      const marker = {}, bad = {toString() { throw marker; }};
      assert(caught(() => RTCStatsReport.prototype.get.call(value, bad)) === marker);
      assert(caught(() => child.RTCStatsReport.prototype.get.call(value, Symbol())) instanceof child.TypeError);
    } finally { pc.close(); }
  });
  await check('cross realm selector validation uses native track identity and callee errors', async () => {
    const pc = new child.RTCPeerConnection();
    try {
      const track = pc.addTransceiver('audio').receiver.track;
      assert((await RTCPeerConnection.prototype.getStats.call(pc, track)) instanceof RTCStatsReport);
      await rejected(window, () => RTCPeerConnection.prototype.getStats.call(pc, new Proxy(track, {})), TypeError);
      await rejected(child, () => child.RTCPeerConnection.prototype.getStats.call({}, track), child.TypeError);
    } finally { pc.close(); }
  });
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
