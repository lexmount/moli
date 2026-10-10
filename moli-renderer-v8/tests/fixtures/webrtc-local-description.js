(async () => {
  const checks = [];
  const assert = (value, message = 'assertion failed') => { if (!value) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error), stack: error.stack}); }
  };
  for (const [realm, w] of [['main', window], ['iframe', document.querySelector('iframe').contentWindow]]) {
    const run = (name, body) => check(`${realm}: ${name}`, async () => {
      const pc = new w.RTCPeerConnection();
      try { await body(pc); } finally { pc.close(); }
    });
    const rejects = async (promise, name) => {
      let error;
      try { await promise; } catch (caught) { error = caught; }
      assert(error instanceof w.DOMException && error.name === name, `expected ${name}, got ${error}`);
      return error;
    };
    const pristine = pc => assert(pc.signalingState === 'stable' && pc.localDescription === null && pc.pendingLocalDescription === null && pc.currentLocalDescription === null);
    await run('uncached explicit SDP rejects without state changes', async pc => {
      await rejects(pc.setLocalDescription({type: 'offer', sdp: 'not SDP'}), 'InvalidModificationError');
      pristine(pc);
    });
    await run('modified latest offer rejects before applying state', async pc => {
      pc.addTransceiver('audio');
      const offer = await pc.createOffer();
      await rejects(pc.setLocalDescription({...offer, sdp: offer.sdp + 'a=x-extension:yes\r\n'}), 'InvalidModificationError');
      pristine(pc);
    });
    await run('explicit empty offer generates valid session', async pc => {
      await pc.setLocalDescription({type: 'offer', sdp: ''});
      assert(pc.signalingState === 'have-local-offer');
      assert(pc.localDescription instanceof w.RTCSessionDescription && pc.localDescription.sdp.startsWith('v=0\r\n'));
      assert(!pc.localDescription.sdp.includes('a=group:BUNDLE \r\n'));
    });
    await run('generated empty session omits empty BUNDLE', async pc => {
      const offer = await pc.createOffer();
      assert(!offer.sdp.includes('a=group:BUNDLE'));
      await pc.setLocalDescription(offer);
      assert(pc.pendingLocalDescription.sdp === offer.sdp);
    });
    await run('latest offer is preserved verbatim in native description', async pc => {
      pc.addTransceiver('audio'); pc.addTransceiver('video');
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      assert(pc.localDescription instanceof w.RTCSessionDescription);
      assert(pc.localDescription.sdp === offer.sdp && pc.pendingLocalDescription.sdp === offer.sdp);
      assert(pc.currentLocalDescription === null);
    });
    await run('older offer rejects after topology changes', async pc => {
      pc.addTransceiver('audio');
      const old = await pc.createOffer();
      pc.addTransceiver('video');
      const latest = await pc.createOffer();
      assert(latest.sdp !== old.sdp);
      await rejects(pc.setLocalDescription(old), 'InvalidModificationError');
      pristine(pc);
      await pc.setLocalDescription(latest);
      assert(pc.localDescription.sdp === latest.sdp);
    });
    await run('offer comparison runs when operation reaches chain head', async pc => {
      pc.addTransceiver('audio');
      const old = await pc.createOffer();
      pc.addTransceiver('video');
      const creating = pc.createOffer();
      const applying = rejects(pc.setLocalDescription(old), 'InvalidModificationError');
      await creating; await applying;
      pristine(pc);
    });
    await run('implicit local offer retains legacy receive options', async pc => {
      const offer = await pc.createOffer({offerToReceiveAudio: true});
      await pc.setLocalDescription();
      assert(offer.sdp.includes('m=audio') && pc.localDescription.sdp.includes('m=audio'));
    });
    await run('implicit local offer reflects current transceivers', async pc => {
      await pc.createOffer();
      pc.addTransceiver('video');
      await pc.setLocalDescription();
      assert(pc.localDescription.sdp.includes('m=video'));
    });
    await run('rollback ignores invalid SDP and clears pending description', async pc => {
      pc.addTransceiver('audio');
      await pc.setLocalDescription();
      await pc.setLocalDescription({type: 'rollback', sdp: '!<Invalid SDP Content>;'});
      pristine(pc);
    });
    await run('rollback in stable rejects without parsing SDP', async pc => {
      await rejects(pc.setLocalDescription({type: 'rollback', sdp: '!invalid'}), 'InvalidStateError');
      pristine(pc);
    });
    await run('answer SDP provenance precedes signaling state checks', async pc => {
      await rejects(pc.setLocalDescription({type: 'answer', sdp: 'not SDP'}), 'InvalidModificationError');
      await rejects(pc.setLocalDescription({type: 'pranswer', sdp: 'not SDP'}), 'InvalidModificationError');
      await rejects(pc.setLocalDescription({type: 'answer'}), 'InvalidStateError');
      pristine(pc);
    });
    await run('dictionary reads once and preserves captured SDP', async pc => {
      pc.addTransceiver('audio');
      const offer = await pc.createOffer();
      const reads = [];
      const promise = pc.setLocalDescription({get sdp() { reads.push('sdp'); return offer.sdp; }, get type() { reads.push('type'); return 'offer'; }});
      assert(reads.join(',') === 'sdp,type' && pc.localDescription === null);
      await promise;
      assert(reads.join(',') === 'sdp,type' && pc.localDescription.sdp === offer.sdp);
    });
    await run('failed update preserves pending description identity', async pc => {
      const offer = await pc.createOffer(); await pc.setLocalDescription(offer);
      const before = pc.localDescription;
      await rejects(pc.setLocalDescription({type: 'offer', sdp: 'modified'}), 'InvalidModificationError');
      assert(pc.localDescription === before && pc.pendingLocalDescription === before && pc.signalingState === 'have-local-offer');
    });
    await run('failed operation does not block next valid operation', async pc => {
      pc.addTransceiver('audio'); const offer = await pc.createOffer();
      const bad = rejects(pc.setLocalDescription({...offer, sdp: 'invalid'}), 'InvalidModificationError');
      const good = pc.setLocalDescription(offer);
      await bad; await good;
      assert(pc.localDescription.sdp === offer.sdp);
    });
    await run('local description errors ignore replaced global constructors', async pc => {
      const original = w.DOMException, originalRTC = w.RTCError;
      try {
        w.DOMException = function() { throw Error('author DOMException constructor'); };
        w.RTCError = function() { throw Error('author RTCError constructor'); };
        let error;
        try { await pc.setLocalDescription({type: 'offer', sdp: 'invalid'}); } catch (caught) { error = caught; }
        assert(error instanceof original && error.name === 'InvalidModificationError');
      } finally { w.DOMException = original; w.RTCError = originalRTC; }
      pristine(pc);
    });
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
})().catch(error => { globalThis.__uiEventResults = {complete: true, total: 1, passed: 0, checks: [{name: 'fixture', passed: false, error: String(error)}]}; });
