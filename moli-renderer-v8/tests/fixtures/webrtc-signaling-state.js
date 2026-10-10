(async () => {
  const checks = [];
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const check = async (name, body) => {
    try { await body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error), stack: error.stack}); }
  };
  const descriptions = ['localDescription', 'currentLocalDescription', 'pendingLocalDescription', 'remoteDescription', 'currentRemoteDescription', 'pendingRemoteDescription'];
  const child = document.querySelector('iframe').contentWindow;
  for (const [realm, w] of [['main', window], ['iframe', child]]) {
    const run = (name, body) => check(`${realm}: ${name}`, async () => {
      const pc = new w.RTCPeerConnection();
      try { await body(pc); } finally { pc.close(); }
    });
    const pristine = pc => assert(pc.signalingState === 'stable' && descriptions.every(name => pc[name] === null));
    await run('all description getters begin at null', pc => { pristine(pc); assert(pc.onsignalingstatechange === null); });
    await run('remote descriptors are readonly native accessors', pc => {
      for (const name of descriptions.slice(3)) {
        const d = w.Object.getOwnPropertyDescriptor(w.RTCPeerConnection.prototype, name);
        assert(typeof d.get === 'function' && d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable);
        assert(!w.Object.hasOwn(pc, name) && !w.Reflect.set(pc, name, {}) && pc[name] === null);
      }
    });
    await run('nonobject handler assignments clear the handler', pc => {
      for (const value of [undefined, null, false, true, 0, 'callback', Symbol('callback'), 1n]) {
        pc.onsignalingstatechange = () => {}; pc.onsignalingstatechange = value;
        assert(pc.onsignalingstatechange === null);
      }
    });
    await run('handler callback object is retained without coercion', pc => {
      let reads = 0; const value = new Proxy({}, {get() { reads++; throw Error('getter'); }});
      pc.onsignalingstatechange = value; assert(pc.onsignalingstatechange === value && reads === 0);
      pc.onsignalingstatechange = null;
    });
    await run('local description state waits for a networking task', async pc => {
      let count = 0; pc.onsignalingstatechange = () => count++;
      const done = pc.setLocalDescription(); pristine(pc); assert(count === 0);
      await Promise.resolve(); pristine(pc); assert(count === 0);
      await done; assert(count === 1 && pc.signalingState === 'have-local-offer');
    });
    await run('description slots update before signaling callback', async pc => {
      let snapshot; pc.onsignalingstatechange = () => { snapshot = [pc.signalingState, pc.localDescription, pc.pendingLocalDescription, pc.currentLocalDescription, pc.remoteDescription, pc.currentRemoteDescription, pc.pendingRemoteDescription]; };
      await pc.setLocalDescription();
      assert(snapshot[0] === 'have-local-offer' && snapshot[1] === pc.localDescription && snapshot[1] === snapshot[2] && snapshot[1] instanceof w.RTCSessionDescription && snapshot.slice(3).every(value => value === null));
    });
    await run('trusted event belongs to target with standard flags', async pc => {
      let event; pc.addEventListener('signalingstatechange', function(e) { event = e; assert(this === pc && e.currentTarget === pc && e.eventPhase === 2); });
      await pc.setLocalDescription();
      assert(event instanceof w.Event && event.type === 'signalingstatechange' && event.isTrusted && event.target === pc);
      assert(!event.bubbles && !event.cancelable && !event.composed && event.currentTarget === null && event.eventPhase === 0);
    });
    await run('handler shares listener registration order', async pc => {
      const order = []; pc.addEventListener('signalingstatechange', () => order.push('before'));
      pc.onsignalingstatechange = () => order.push('handler');
      pc.addEventListener('signalingstatechange', () => order.push('after'));
      await pc.setLocalDescription().then(() => order.push('promise'));
      assert(order.join(',') === 'before,handler,after,promise', order.join(','));
    });
    await run('replacing handler retains registration position', async pc => {
      const order = []; pc.addEventListener('signalingstatechange', () => order.push('before'));
      pc.onsignalingstatechange = () => order.push('old');
      pc.addEventListener('signalingstatechange', () => order.push('after'));
      pc.onsignalingstatechange = () => order.push('new');
      await pc.setLocalDescription(); assert(order.join(',') === 'before,new,after');
    });
    await run('cleared handler is appended when reactivated', async pc => {
      const order = []; pc.onsignalingstatechange = () => order.push('old');
      pc.addEventListener('signalingstatechange', () => order.push('listener'));
      pc.onsignalingstatechange = null; pc.onsignalingstatechange = () => order.push('new');
      await pc.setLocalDescription(); assert(order.join(',') === 'listener,new');
    });
    await run('same state local offers do not refire event', async pc => {
      let count = 0; pc.onsignalingstatechange = () => count++;
      const offer = await pc.createOffer(); await pc.setLocalDescription(offer);
      await pc.setLocalDescription(offer); await pc.setLocalDescription();
      assert(count === 1 && pc.localDescription.type === 'offer');
    });
    await run('rollback clears pending before firing stable', async pc => {
      await pc.setLocalDescription(); let snapshot;
      pc.onsignalingstatechange = () => { snapshot = [pc.signalingState, ...descriptions.map(name => pc[name])]; };
      await pc.setLocalDescription({type: 'rollback', sdp: '!invalid SDP'});
      assert(snapshot[0] === 'stable' && snapshot.slice(1).every(value => value === null)); pristine(pc);
    });
    await run('queued offer and rollback preserve event promise barriers', async pc => {
      const order = []; pc.onsignalingstatechange = () => order.push(`event:${pc.signalingState}`);
      const offer = pc.setLocalDescription().then(() => order.push('offer'));
      const rollback = pc.setLocalDescription({type: 'rollback'}).then(() => order.push('rollback'));
      pristine(pc); await Promise.all([offer, rollback]);
      assert(order.join(',') === 'event:have-local-offer,offer,event:stable,rollback', order.join(',')); pristine(pc);
    });
    await run('reentrant rollback joins the same operations chain', async pc => {
      const order = []; let rollback;
      pc.onsignalingstatechange = () => { order.push(`event:${pc.signalingState}`); if (pc.signalingState === 'have-local-offer') rollback = pc.setLocalDescription({type: 'rollback'}).then(() => order.push('rollback')); };
      await pc.setLocalDescription().then(() => order.push('offer')); await rollback;
      assert(order.join(',') === 'event:have-local-offer,offer,event:stable,rollback', order.join(',')); pristine(pc);
    });
    await run('once listener runs only for first transition', async pc => {
      let count = 0; pc.addEventListener('signalingstatechange', () => count++, {once: true});
      await pc.setLocalDescription(); await pc.setLocalDescription({type: 'rollback'}); assert(count === 1);
    });
    await run('aborted listener is not invoked', async pc => {
      let count = 0; const controller = new w.AbortController();
      pc.addEventListener('signalingstatechange', () => count++, {signal: controller.signal}); controller.abort();
      await pc.setLocalDescription(); assert(count === 0);
    });
    await run('removed listener stays absent on subsequent state', async pc => {
      let count = 0; const callback = () => count++; pc.addEventListener('signalingstatechange', callback);
      await pc.setLocalDescription(); pc.removeEventListener('signalingstatechange', callback);
      await pc.setLocalDescription({type: 'rollback'}); assert(count === 1);
    });
    await run('stopImmediatePropagation does not cancel operation', async pc => {
      const order = []; pc.addEventListener('signalingstatechange', e => { order.push('first'); e.stopImmediatePropagation(); e.preventDefault(); });
      pc.onsignalingstatechange = () => order.push('handler'); pc.addEventListener('signalingstatechange', () => order.push('last'));
      await pc.setLocalDescription(); assert(order.join(',') === 'first' && pc.signalingState === 'have-local-offer');
    });
    await run('listener exception does not reject description operation', async pc => {
      let after = false; pc.onsignalingstatechange = () => { throw Error('expected signaling listener error'); };
      pc.addEventListener('signalingstatechange', () => after = true);
      await pc.setLocalDescription(); assert(after && pc.signalingState === 'have-local-offer');
    });
    await run('manual dispatch is untrusted and leaves state intact', pc => {
      let trusted; pc.onsignalingstatechange = event => trusted = event.isTrusted;
      assert(pc.dispatchEvent(new w.Event('signalingstatechange'))); assert(trusted === false); pristine(pc);
    });
    await run('native event construction ignores replaced Event global', async pc => {
      const OriginalEvent = w.Event; let event;
      pc.onsignalingstatechange = e => event = e;
      try { w.Event = function() { throw Error('author Event constructor'); }; await pc.setLocalDescription(); }
      finally { w.Event = OriginalEvent; }
      assert(event instanceof OriginalEvent && event.isTrusted);
    });
    await run('failed transition neither changes slots nor fires event', async pc => {
      let count = 0, error; pc.onsignalingstatechange = () => count++;
      try { await pc.setLocalDescription({type: 'rollback'}); } catch (caught) { error = caught; }
      assert(error instanceof w.DOMException && error.name === 'InvalidStateError' && count === 0); pristine(pc);
    });
    await run('native brands reject forged and proxy receivers before traps', pc => {
      let traps = 0; const revoked = w.Proxy.revocable(pc, {}); revoked.revoke();
      const invalid = [{}, w.Object.create(w.RTCPeerConnection.prototype), w.Object.create(pc), new w.Proxy(pc, {get() { traps++; throw Error('trap'); }}), revoked.proxy];
      for (const name of [...descriptions, 'onsignalingstatechange']) {
        const d = w.Object.getOwnPropertyDescriptor(w.RTCPeerConnection.prototype, name);
        for (const value of invalid) {
          let error; try { d.get.call(value); } catch (caught) { error = caught; }
          assert(error instanceof w.TypeError);
          if (d.set) { error = undefined; try { d.set.call(value, () => {}); } catch (caught) { error = caught; } assert(error instanceof w.TypeError); }
        }
      }
      assert(traps === 0);
    });
    await run('close before task aborts event and promise completion', async pc => {
      let events = 0, settled = false; pc.onsignalingstatechange = () => events++;
      pc.setLocalDescription().then(() => settled = true, () => settled = true); pc.close();
      await new Promise(resolve => w.setTimeout(resolve, 0));
      assert(pc.signalingState === 'closed' && events === 0 && !settled && descriptions.every(name => pc[name] === null));
    });
    await run('close during event leaves operation promise pending', async pc => {
      let settled = false, event, fired;
      const observed = new Promise(resolve => fired = resolve);
      pc.onsignalingstatechange = e => { event = e; pc.close(); fired(); };
      pc.setLocalDescription().then(() => settled = true, () => settled = true);
      await observed; await new Promise(resolve => w.setTimeout(resolve, 0));
      assert(event.isTrusted && pc.signalingState === 'closed' && !settled && pc.localDescription.type === 'offer');
    });
  }
  for (const [label, owner, callee] of [['main owner', window, child], ['iframe owner', child, window]]) {
    await check(`cross realm: ${label} gets owner Event`, async () => {
      const pc = new owner.RTCPeerConnection(); let event;
      pc.onsignalingstatechange = e => event = e;
      try {
        await callee.RTCPeerConnection.prototype.setLocalDescription.call(pc);
        assert(event instanceof owner.Event && !(event instanceof callee.Event) && event.target === pc && event.isTrusted);
        for (const name of descriptions) assert(callee.Object.getOwnPropertyDescriptor(callee.RTCPeerConnection.prototype, name).get.call(pc) === pc[name]);
      } finally { pc.close(); }
    });
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})().catch(error => { globalThis.__uiEventResults = {complete: true, total: 1, passed: 0, checks: [{name: 'fixture', passed: false, error: String(error)}]}; return false; });
