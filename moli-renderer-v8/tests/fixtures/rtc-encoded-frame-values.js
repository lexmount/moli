(() => {
  const assert = (value, name) => { if (!value) throw Error(name); };
  const other = document.querySelector('iframe').contentWindow;
  const base = ['captureTime', 'contributingSources', 'mimeType', 'payloadType', 'receiveTime', 'rtpTimestamp', 'senderCaptureTimeOffset', 'synchronizationSource'];
  for (const [name, original, nativeProxy, own] of [
    ['RTCEncodedAudioFrame', audioFrame, nativeaudioFrame, ['audioLevel', 'sequenceNumber']],
    ['RTCEncodedVideoFrame', videoFrame, nativevideoFrame, ['dependencies', 'frameId', 'height', 'spatialIndex', 'temporalIndex', 'timestamp', 'width']]
  ]) {
    const C = window[name], childC = other[name];
    class DerivedFrame extends C {}
    assert(new DerivedFrame(original) instanceof DerivedFrame, name + ' preserves newTarget prototype');
    const d = Object.getOwnPropertyDescriptor(C.prototype, 'data');
    const metadata = C.prototype.getMetadata.call(original);
    const data = d.get.call(original);
    assert(data === d.get.call(original) && data.byteLength === 4, name + ' data identity');
    assert(d.get.call(nativeProxy) === data, name + ' native proxy identity');
    const clone = new childC(nativeProxy);
    assert(clone instanceof childC && clone.data instanceof other.ArrayBuffer && clone.data !== data, name + ' clone realm and buffer isolation');
    assert(JSON.stringify(clone.getMetadata()) === JSON.stringify(metadata), name + ' copied metadata');
    assert(clone.getMetadata() instanceof other.Object && clone.getMetadata().contributingSources instanceof other.Array, name + ' metadata realm');
    assert(childC.prototype.getMetadata.call(original) instanceof other.Object, name + ' borrowed metadata callee realm');
    new Uint8Array(clone.data)[0] = 255;
    assert(new Uint8Array(data)[0] === 1, name + ' copy bytes independent');
    clone.getMetadata().contributingSources.push(999);
    assert(clone.getMetadata().contributingSources.length === 2, name + ' returned sequences independent');
    if ('dependencies' in metadata) {
      clone.getMetadata().dependencies.push(999);
      assert(clone.getMetadata().dependencies.length === 2, name + ' dependency isolation');
      assert(clone.type === 'key', name + ' type copied');
    }
    const replacement = new other.ArrayBuffer(3);
    d.set.call(nativeProxy, replacement);
    assert(d.get.call(original) === replacement, name + ' setter retains input identity and realm');
    for (const invalid of [undefined, null, {}, new Uint8Array(1), new Proxy(new ArrayBuffer(1), {}), new ArrayBuffer(1, {maxByteLength: 2})]) {
      let error;
      try { d.set.call(original, invalid); } catch (caught) { error = caught; }
      assert(error instanceof TypeError && d.get.call(original) === replacement, name + ' invalid data leaves previous value');
    }
    if (typeof SharedArrayBuffer === 'function') {
      let error; try { d.set.call(original, new SharedArrayBuffer(1)); } catch (caught) { error = caught; }
      assert(error instanceof TypeError, name + ' nonshared data');
    }
    const detached = new ArrayBuffer(1); structuredClone(detached, {transfer: [detached]});
    d.set.call(original, detached);
    assert(d.get.call(original) === detached, name + ' ArrayBuffer conversion retains detached identity');
    let error; try { new C(original); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, name + ' CloneArrayBuffer rejects detached data');
    d.set.call(original, replacement);
    const conversionOrder = [];
    const overrides = new Proxy({}, {get(target, key) {conversionOrder.push(key); return undefined;}});
    new C(original, {metadata: overrides});
    assert(conversionOrder.join() === [...base, ...own].join(), name + ' inheritance conversion order');
    for (const member of [...base, ...own]) {
      const sentinel = {};
      const order = [];
      const options = new Proxy({}, {get(target, key) {order.push(key); if (key === member) throw sentinel; return undefined;}});
      error = undefined;
      try { new C(original, {metadata: options}); } catch (caught) { error = caught; }
      assert(error === sentinel && order.at(-1) === member, name + ' conversion short circuit ' + member);
    }
    const replacement2 = new Uint8Array([9, 8]).buffer;
    const updated = new C(original, {get metadata() {
      d.set.call(original, replacement2);
      return {rtpTimestamp: -1, contributingSources: [-1, 4294967297], mimeType: 'x\ud800', payloadType: 257};
    }});
    assert(Array.from(new Uint8Array(updated.data)).join() === '9,8', name + ' data snapshot after conversion');
    const out = updated.getMetadata();
    assert(out.rtpTimestamp === 4294967295 && out.payloadType === 1 && out.mimeType === 'x\ud800', name + ' numeric wrap and UTF16');
    assert(out.contributingSources.join() === '4294967295,1', name + ' sequence conversion');
    const empty = new C(original, {metadata: null});
    assert(JSON.stringify(empty.getMetadata()) === JSON.stringify(C.prototype.getMetadata.call(original)), name + ' null metadata leaves fields');
    assert(new C(original, {metadata: {rtpTimestamp: undefined}}).getMetadata().rtpTimestamp === metadata.rtpTimestamp, name + ' undefined member preserves original');
    assert(new C(original, {metadata: {rtpTimestamp: null}}).getMetadata().rtpTimestamp === 0, name + ' null numeric member converts to zero');
    for (const invalid of [Infinity, NaN, -Infinity, Symbol(), 1n]) {
      error = undefined;
      try { new C(original, {metadata: {captureTime: invalid}}); } catch (caught) { error = caught; }
      assert(error instanceof TypeError, name + ' finite timestamp conversion rejects invalid values');
    }
    error = undefined;
    try { new childC(original, {metadata: {captureTime: Infinity}}); } catch (caught) { error = caught; }
    assert(error instanceof other.TypeError, name + ' metadata conversion error callee realm');
    const retained = d.get.call(original);
    error = undefined;
    try { new C(original, {get metadata() { structuredClone(retained, {transfer: [retained]}); return {}; }}); } catch (caught) { error = caught; }
    assert(error instanceof TypeError && d.get.call(original) === retained && retained.byteLength === 0, name + ' detachment during conversion detected');
    d.set.call(original, new Uint8Array([9, 8]).buffer);
    let traps = 0;
    const authorProxy = new Proxy(original, {get() {traps++; throw Error('trap');}, getPrototypeOf() {traps++; throw Error('trap');}});
    for (const receiver of [authorProxy, Object.create(original)]) {
      for (const fn of [d.get, d.set, C.prototype.getMetadata]) {
        error = undefined;
        try { fn.call(receiver, replacement2); } catch (caught) { error = caught; }
        assert(error instanceof TypeError && traps === 0, name + ' author receivers rejected without traps');
      }
    }
    for (const key of ['data', 'getMetadata', 'type']) Object.defineProperty(original, key, {get() {throw Error('author getter: '+key);}, configurable: true});
    const internal = new C(original);
    assert(internal.data.byteLength === 2 && internal.getMetadata().rtpTimestamp === metadata.rtpTimestamp, name + ' public original properties ignored');
    const previous = Object.getOwnPropertyDescriptor(Object.prototype, 'receiveTime');
    Object.defineProperty(Object.prototype, 'receiveTime', {get() {throw Error('inherited metadata getter');}, configurable: true});
    try { assert(!Object.hasOwn(internal.getMetadata(), 'receiveTime'), name + ' absent internal field ignores pollution'); }
    finally { if (previous) Object.defineProperty(Object.prototype, 'receiveTime', previous); else delete Object.prototype.receiveTime; }
    const before = C.prototype.getMetadata.call(original).contributingSources.join();
    Object.defineProperty(Array.prototype, '0', {set() {throw Error('inherited sequence setter');}, configurable: true});
    try {
      assert(C.prototype.getMetadata.call(original).contributingSources.join() === before, name + ' result sequence data properties');
    } finally { delete Array.prototype[0]; }
  }
  return true;
})()
