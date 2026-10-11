(async () => {
  const checks = [];
  const connections = [];
  const ensure = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const equal = (actual, expected, message) => ensure(Object.is(actual, expected), message);
  const connection = (realm = globalThis, configuration) => {
    const pc = new realm.RTCPeerConnection(configuration);
    connections.push(pc);
    return pc;
  };
  const associated = async (realm = globalThis, configuration) => {
    const pc = connection(realm, configuration);
    const channel = pc.createDataChannel('data');
    await pc.setLocalDescription();
    ensure(pc.sctp instanceof realm.RTCSctpTransport, 'native SCTP association');
    return {pc, channel, sctp: pc.sctp};
  };
  const check = async (name, body) => {
    try {
      await body();
      checks.push({name, passed: true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error)});
    }
  };
  const rejectsReceiver = (fn, receiver, realm = globalThis) => {
    let error;
    try { fn.call(receiver); } catch (caught) { error = caught; }
    ensure(error instanceof realm.TypeError, 'receiver must throw the callee realm TypeError');
  };

  await check('SCTP is an illegal constructor with EventTarget inheritance', () => {
    let error;
    try { new RTCSctpTransport(); } catch (caught) { error = caught; }
    ensure(error instanceof TypeError, 'SCTP cannot be constructed by authors');
    equal(Object.getPrototypeOf(RTCSctpTransport.prototype), EventTarget.prototype, 'EventTarget parent');
  });
  await check('SCTP attributes and pc.sctp have WebIDL prototype descriptors', () => {
    for (const key of ['transport', 'state', 'maxMessageSize', 'maxChannels', 'onstatechange']) {
      const descriptor = Object.getOwnPropertyDescriptor(RTCSctpTransport.prototype, key);
      ensure(descriptor?.enumerable && descriptor.configurable && typeof descriptor.get === 'function', key);
      equal(typeof descriptor.set, key === 'onstatechange' ? 'function' : 'undefined', 'setter ' + key);
    }
    const descriptor = Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype, 'sctp');
    ensure(descriptor?.enumerable && descriptor.configurable && typeof descriptor.get === 'function' && descriptor.set === undefined, 'pc.sctp descriptor');
  });
  await check('Creating a DataChannel keeps pc.sctp null', () => {
    const pc = connection();
    equal(pc.sctp, null, 'initial association');
    pc.createDataChannel('data');
    equal(pc.sctp, null, 'unapplied data channel');
  });
  await check('Creating an offer keeps pc.sctp null', async () => {
    const pc = connection();
    pc.createDataChannel('data');
    const offer = await pc.createOffer();
    ensure(offer.sdp.includes('m=application'), 'data section');
    equal(pc.sctp, null, 'unapplied offer');
  });
  await check('An applied local data offer exposes native SCTP DTLS and ICE', async () => {
    const {sctp} = await associated();
    ensure(sctp instanceof EventTarget && sctp.transport instanceof RTCDtlsTransport, 'native SCTP and DTLS');
    ensure(sctp.transport.iceTransport instanceof RTCIceTransport, 'native ICE');
    equal(sctp.transport, sctp.transport, 'stable transport identity');
    equal(Object.prototype.toString.call(sctp), '[object RTCSctpTransport]', 'class string');
  });
  await check('An unnegotiated SCTP association has null message and channel limits', async () => {
    const {sctp} = await associated();
    equal(sctp.state, 'connecting', 'initial SCTP state');
    equal(sctp.maxMessageSize, null, 'no negotiated message size');
    equal(sctp.maxChannels, null, 'no negotiated stream count');
  });
  await check('Local association does not fabricate a transport handshake or channel ID', async () => {
    const {pc, channel, sctp} = await associated();
    equal(channel.readyState, 'connecting', 'data channel is pending');
    equal(channel.id, null, 'unassigned channel id');
    equal(sctp.transport.state, 'new', 'DTLS has not connected');
    equal(sctp.transport.iceTransport.state, 'new', 'ICE has not connected');
    equal(pc.iceGatheringState, 'new', 'no fabricated candidates');
    equal(pc.remoteDescription, null, 'no remote description');
  });
  await check('Repeated local offers retain the SCTP and DTLS identities', async () => {
    const {pc, sctp} = await associated();
    const dtls = sctp.transport;
    const offer = await pc.createOffer();
    equal(pc.sctp, sctp, 'createOffer preserves association');
    await pc.setLocalDescription(offer);
    equal(pc.sctp, sctp, 'reoffer preserves SCTP');
    equal(sctp.transport, dtls, 'reoffer preserves DTLS');
  });
  await check('Rollback clears pc.sctp and closes the abandoned association', async () => {
    const {pc, sctp} = await associated();
    await pc.setLocalDescription({type: 'rollback'});
    equal(pc.sctp, null, 'rollback association');
    equal(sctp.state, 'closed', 'abandoned SCTP');
    equal(sctp.transport.state, 'closed', 'abandoned DTLS');
  });
  await check('A retained offer creates fresh SCTP and DTLS after rollback', async () => {
    const pc = connection();
    pc.createDataChannel('data');
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    const old = pc.sctp;
    await pc.setLocalDescription({type: 'rollback'});
    await pc.setLocalDescription(offer);
    ensure(pc.sctp !== old && pc.sctp.transport !== old.transport, 'closed objects cannot be reused');
    equal(pc.sctp.state, 'connecting', 'fresh SCTP');
    equal(pc.sctp.maxMessageSize, null, 'fresh message limit');
    equal(old.state, 'closed', 'old object remains closed');
  });
  await check('Fresh offers after rollback retain pending DataChannels', async () => {
    const {pc, channel, sctp} = await associated();
    await pc.setLocalDescription({type: 'rollback'});
    await pc.setLocalDescription();
    ensure(pc.sctp instanceof RTCSctpTransport && pc.sctp !== sctp, 'new association');
    equal(pc.sctp.maxChannels, null, 'fresh channel limit');
    equal(channel.readyState, 'connecting', 'pending original channel');
  });
  await check('An older RTP offer does not incorporate a subsequently created DataChannel', async () => {
    const pc = connection();
    const transceiver = pc.addTransceiver('audio');
    const offer = await pc.createOffer();
    pc.createDataChannel('later');
    await pc.setLocalDescription(offer);
    equal(pc.sctp, null, 'old snapshot has no data section');
    ensure(transceiver.sender.transport instanceof RTCDtlsTransport, 'RTP association still applies');
    await pc.setLocalDescription();
    ensure(pc.sctp instanceof RTCSctpTransport, 'new snapshot includes data');
  });
  await check('An older legacy receive offer does not incorporate a later DataChannel', async () => {
    const pc = connection();
    const offer = await pc.createOffer({offerToReceiveAudio: true});
    pc.createDataChannel('later');
    await pc.setLocalDescription(offer);
    equal(pc.sctp, null, 'legacy snapshot has no data section');
    await pc.setLocalDescription(await pc.createOffer({offerToReceiveAudio: true}));
    ensure(pc.sctp instanceof RTCSctpTransport, 'new legacy snapshot includes data');
  });
  await check('Legacy receive offers with data expose a native SCTP transport', async () => {
    const pc = connection();
    pc.createDataChannel('data');
    const offer = await pc.createOffer({offerToReceiveAudio: true, offerToReceiveVideo: true});
    await pc.setLocalDescription(offer);
    ensure(pc.sctp instanceof RTCSctpTransport, 'legacy data association');
    ensure(pc.sctp.transport instanceof RTCDtlsTransport, 'legacy data DTLS');
    ensure(pc.sctp.transport.iceTransport instanceof RTCIceTransport, 'legacy data ICE');
    ensure(offer.sdp.includes('m=application') && offer.sdp.includes('a=ice-ufrag:'), 'offered data section');
  });
  for (const bundlePolicy of ['balanced', 'max-compat', 'max-bundle']) {
    await check('SCTP follows the ' + bundlePolicy + ' transport plan', async () => {
      const pc = connection(globalThis, {bundlePolicy});
      const audio = pc.addTransceiver('audio');
      const video = pc.addTransceiver('video');
      pc.createDataChannel('data');
      await pc.setLocalDescription();
      const dtls = pc.sctp.transport;
      ensure(dtls instanceof RTCDtlsTransport, 'SCTP DTLS');
      equal(dtls === audio.sender.transport, bundlePolicy === 'max-bundle', 'audio bundling');
      equal(dtls === video.sender.transport, bundlePolicy === 'max-bundle', 'video bundling');
    });
  }
  await check('Connections never share their SCTP or DTLS objects', async () => {
    const first = await associated();
    const second = await associated();
    ensure(first.sctp !== second.sctp && first.sctp.transport !== second.sctp.transport, 'connection ownership');
    first.pc.close();
    equal(second.sctp.state, 'connecting', 'independent lifetime');
  });
  await check('SCTP statechange handlers keep their EventTarget registration order', async () => {
    const {sctp} = await associated();
    const trace = [];
    sctp.addEventListener('statechange', () => trace.push('before'));
    sctp.onstatechange = () => trace.push('old');
    sctp.addEventListener('statechange', () => trace.push('after'));
    sctp.onstatechange = () => trace.push('replacement');
    sctp.dispatchEvent(new Event('statechange'));
    equal(trace.join('|'), 'before|replacement|after', 'replace handler in place');
    sctp.onstatechange = null;
    sctp.onstatechange = () => trace.push('reinstalled');
    trace.length = 0;
    sctp.dispatchEvent(new Event('statechange'));
    equal(trace.join('|'), 'before|after|reinstalled', 'new registration position');
    sctp.onstatechange = null;
  });
  await check('SCTP handler conversion does not inspect callback objects', async () => {
    const {sctp} = await associated();
    let reads = 0;
    const value = {get handleEvent() {reads++; throw new Error('unexpected callback property access');}};
    sctp.onstatechange = value;
    equal(sctp.onstatechange, value, 'callback object identity');
    sctp.dispatchEvent(new Event('statechange'));
    equal(reads, 0, 'EventHandler is not an EventListener object');
    for (const primitive of [undefined, null, false, 1, 'handler']) {
      sctp.onstatechange = primitive;
      equal(sctp.onstatechange, null, 'non-object clears handler');
    }
  });
  await check('SCTP close is synchronous retains pc.sctp and notifies exactly once', async () => {
    const {pc, channel, sctp} = await associated();
    const trace = [];
    sctp.onstatechange = event => {
      trace.push({trusted:event.isTrusted, target:event.target===sctp, current:event.currentTarget===sctp,
        bubbles:event.bubbles, cancelable:event.cancelable, state:sctp.state, signaling:pc.signalingState,
        ice:pc.iceConnectionState, connection:pc.connectionState, dtls:sctp.transport.state,
        channel:channel.readyState});
      pc.close();
    };
    pc.close();
    equal(pc.sctp, sctp, 'closed association stays observable');
    equal(trace.length, 1, 'single synchronous close notification');
    const event = trace[0];
    ensure(event.trusted && event.target && event.current && !event.bubbles && !event.cancelable, 'native event');
    for (const key of ['state','signaling','channel']) equal(event[key], 'closed', 'SCTP shutdown ' + key);
    for (const key of ['ice','connection','dtls']) equal(event[key], 'new', 'later shutdown step ' + key);
    equal(sctp.transport.state, 'closed', 'DTLS shutdown completes before close returns');
    equal(sctp.transport.iceTransport.state, 'closed', 'ICE shutdown completes before close returns');
    equal(pc.iceConnectionState, 'closed', 'connection ICE shutdown');
    equal(pc.connectionState, 'closed', 'connection shutdown');
    pc.close();
    equal(trace.length, 1, 'idempotent close');
  });
  await check('Rollback delivers one trusted SCTP closure event', async () => {
    const {pc, sctp} = await associated();
    const trace = [];
    sctp.onstatechange = event => trace.push([event.isTrusted, sctp.state, pc.sctp]);
    await pc.setLocalDescription({type:'rollback'});
    await pc.getStats();
    equal(trace.length, 1, 'rollback event');
    ensure(trace[0][0] && trace[0][1] === 'closed', 'retired association notification');
    pc.close();
    equal(trace.length, 1, 'closing a rolled-back connection does not reclose old SCTP');
  });
  await check('Closing an owner after rollback preserves its SCTP closure notification', async () => {
    const {pc,sctp}=await associated();
    const trace=[];
    sctp.onstatechange=event=>trace.push(event.isTrusted && sctp.state==='closed');
    await pc.setLocalDescription({type:'rollback'}).then(()=>pc.close());
    await pc.getStats();
    equal(trace.length,1,'one final closure notification');
    equal(trace[0],true,'trusted final state');
  });
  for (const key of ['transport', 'state', 'maxMessageSize', 'maxChannels']) {
    await check('SCTP ' + key + ' getter rejects forged and author Proxy receivers', async () => {
      const {sctp} = await associated();
      const getter = Object.getOwnPropertyDescriptor(RTCSctpTransport.prototype,key).get;
      let traps = 0;
      const proxy = new Proxy(sctp, {get() {traps++; throw new Error('get trap');}, getPrototypeOf() {traps++; throw new Error('prototype trap');}});
      const revoked = Proxy.revocable(sctp, {}); revoked.revoke();
      for (const receiver of [null, undefined, {}, Object.create(RTCSctpTransport.prototype), Object.create(sctp), proxy, revoked.proxy]) rejectsReceiver(getter,receiver);
      equal(traps, 0, 'brand checking bypasses author traps');
    });
  }
  await check('pc.sctp rejects forged and author Proxy receivers', () => {
    const pc = connection();
    const getter = Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype,'sctp').get;
    for (const receiver of [{}, Object.create(pc), new Proxy(pc,{})]) rejectsReceiver(getter,receiver);
  });
  await check('SCTP handler receiver checking precedes callback access', async () => {
    const {sctp} = await associated();
    const descriptor = Object.getOwnPropertyDescriptor(RTCSctpTransport.prototype,'onstatechange');
    let reads = 0;
    const callback = new Proxy(function(){}, {get(){reads++; throw new Error('callback access');}});
    for (const receiver of [{},Object.create(sctp),new Proxy(sctp,{})]) {
      let error;
      try {descriptor.set.call(receiver,callback);} catch(caught) {error=caught;}
      ensure(error instanceof TypeError, 'setter rejects receiver');
      rejectsReceiver(descriptor.get,receiver);
    }
    equal(reads,0,'invalid receiver does not inspect callback');
  });
  await check('SCTP readonly getters ignore author properties resembling native slots', async () => {
    const {sctp} = await associated();
    equal(Reflect.set(sctp,'state','connected'),false,'readonly attribute');
    sctp.__moliRtcTransportState = 'connected';
    sctp.__moliRtcSctpMaxMessageSize = 123;
    equal(sctp.state,'connecting','private state');
    equal(sctp.maxMessageSize,null,'private message limit');
  });
  await check('Borrowed methods allocate SCTP and transports in the connection realm', async () => {
    const other = document.querySelector('iframe').contentWindow;
    const pc = connection(other);
    RTCPeerConnection.prototype.createDataChannel.call(pc,'data');
    await RTCPeerConnection.prototype.setLocalDescription.call(pc);
    ensure(pc.sctp instanceof other.RTCSctpTransport && !(pc.sctp instanceof RTCSctpTransport),'owner SCTP realm');
    ensure(pc.sctp.transport instanceof other.RTCDtlsTransport,'owner DTLS realm');
    ensure(pc.sctp.transport.iceTransport instanceof other.RTCIceTransport,'owner ICE realm');
    equal(Object.getOwnPropertyDescriptor(RTCSctpTransport.prototype,'state').get.call(pc.sctp),'connecting','genuine cross-realm receiver');
    rejectsReceiver(Object.getOwnPropertyDescriptor(other.RTCSctpTransport.prototype,'state').get,{},other);
    rejectsReceiver(Object.getOwnPropertyDescriptor(other.RTCPeerConnection.prototype,'sctp').get,{},other);
  });
  await check('Borrowed close dispatches SCTP statechange in the owner realm', async () => {
    const other = document.querySelector('iframe').contentWindow;
    const {pc,sctp} = await associated(other);
    let event;
    sctp.onstatechange = value => {event=value;};
    RTCPeerConnection.prototype.close.call(pc);
    ensure(event instanceof other.Event && !(event instanceof Event) && event.isTrusted,'owner event realm');
  });
  await check('SCTP native construction and events survive replaced global constructors', async () => {
    const saved = Object.fromEntries(['RTCSctpTransport','RTCDtlsTransport','RTCIceTransport','Event'].map(key=>[key,globalThis[key]]));
    const pc = connection(); pc.createDataChannel('data');
    try {
      for (const key of Object.keys(saved)) globalThis[key] = function(){throw new Error('replaced ' + key);};
      await pc.setLocalDescription();
      const sctp=pc.sctp;
      ensure(Object.getPrototypeOf(sctp)===saved.RTCSctpTransport.prototype,'intrinsic SCTP prototype');
      ensure(Object.getPrototypeOf(sctp.transport)===saved.RTCDtlsTransport.prototype,'intrinsic DTLS prototype');
      let event;
      sctp.onstatechange=value=>{event=value;};
      pc.close();
      ensure(event instanceof saved.Event && event.isTrusted,'intrinsic Event');
    } finally {Object.assign(globalThis,saved);}
  });
  await check('Invalid local descriptions reject without associating SCTP and the chain recovers', async () => {
    const pc=connection();pc.createDataChannel('data');
    const offer=await pc.createOffer();
    let error;
    try {await pc.setLocalDescription({type:'offer',sdp:'v=0\r\ninvalid\r\n'});} catch(caught){error=caught;}
    ensure(error instanceof DOMException,'invalid description rejection');
    equal(pc.sctp,null,'rejected association');
    equal(pc.signalingState,'stable','rejected signaling state');
    await pc.setLocalDescription(offer);
    ensure(pc.sctp instanceof RTCSctpTransport,'later operation still applies');
  });
  await check('Close before a queued local offer prevents SCTP association and completion', async () => {
    const pc=connection();pc.createDataChannel('data');
    const trace=[];
    pc.setLocalDescription().then(()=>trace.push('resolved'),()=>trace.push('rejected'));
    pc.close();
    await pc.getStats();
    equal(pc.sctp,null,'closed owner cannot acquire SCTP');
    equal(trace.length,0,'aborted operation does not settle');
  });
  for (const pc of connections) pc.close();
  const passed=checks.filter(row=>row.passed).length;
  globalThis.__uiEventResults={complete:true,passed,total:checks.length,checks};
  return passed===checks.length;
})()
