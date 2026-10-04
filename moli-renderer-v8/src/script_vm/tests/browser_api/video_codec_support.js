(async () => {
  async function probe(realm) {
    const assert = (ok, message) => { if (!ok) throw new Error(message); };
    for (const name of ['VideoDecoder', 'VideoEncoder']) {
      const encoder = name === 'VideoEncoder';
      const ctor = realm[name];
      const query = ctor.isConfigSupported;
      const descriptor = Object.getOwnPropertyDescriptor(ctor, 'isConfigSupported');
      assert(typeof query === 'function' && query.length === 1 && descriptor.enumerable && descriptor.writable && descriptor.configurable, name + ' static operation');
      const config = { codec: 'vp8', ...(encoder ? {width: 640, height: 480} : {}) };
      const promise = query.call({}, config);
      assert(promise instanceof realm.Promise, name + ' promise belongs to callee realm');
      const support = await promise;
      assert(support.supported === false && support.config !== config && Object.getPrototypeOf(support.config) === realm.Object.prototype, name + ' honest independent config');
      assert(support.config.codec === 'vp8' && support.config.hardwareAcceleration === 'no-preference', name + ' converted defaults');
      assert(!Object.hasOwn(support.config, encoder ? 'displayWidth' : 'codedWidth'), name + ' absent members stay absent');
      const inherited = Object.create({codec: {toString: () => 'vp8'}});
      if (encoder) { inherited.width = '640.9'; inherited.height = 480; }
      inherited.ignored = 'unknown';
      const inheritedSupport = await query(inherited);
      assert(inheritedSupport.config.codec === 'vp8' && !Object.hasOwn(inheritedSupport.config, 'ignored'), name + ' dictionary conversion and unknown keys');
      if (encoder) assert(inheritedSupport.config.width === 640, 'EnforceRange truncation');
      const invalids = [undefined, null, {}, {...config, codec: ''}, {...config, codec: ' \t\n'},
        {...config, hardwareAcceleration: 'invalid'}, {...config, [encoder ? 'width' : 'codedWidth']: -1},
        {...config, [encoder ? 'height' : 'codedHeight']: Infinity},
        {...config, [encoder ? 'displayWidth' : 'displayAspectWidth']: 0}];
      if (encoder) invalids.push({...config, width: 0}, {...config, alpha: 'opaque'}, {...config, framerate: NaN}, {...config, bitrate: 2n});
      else invalids.push({...config, codedWidth: 12}, {...config, codedWidth: 0, codedHeight: 12}, {...config, colorSpace: {primaries: 'invalid'}}, {...config, description: {}}, {...config, rotation: Infinity});
      for (const invalid of invalids) {
        let rejected;
        let result;
        try { result = query(invalid); } catch (_) { throw Error(name + ' must reject instead of throwing synchronously'); }
        assert(result instanceof realm.Promise, name + ' invalid config returns promise');
        try { await result; } catch (error) { rejected = error; }
        assert(rejected instanceof realm.TypeError, name + ' invalid config rejects in callee realm');
      }
      const sentinel = {};
      let caught;
      try { await query({...config, get codec() { throw sentinel; }}); } catch (error) { caught = error; }
      assert(caught === sentinel, name + ' preserves getter exceptions');
      const reads = [];
      const ordered = new Proxy(config, {get(target, key, receiver) { reads.push(key); return Reflect.get(target, key, receiver); }});
      await query(ordered);
      const expected = encoder ? ['alpha','bitrate','bitrateMode','codec','contentHint','displayHeight','displayWidth','framerate','hardwareAcceleration','height','latencyMode','scalabilityMode','width']
        : ['codedHeight','codedWidth','codec','colorSpace','description','displayAspectHeight','displayAspectWidth','flip','hardwareAcceleration','optimizeForLatency','rotation'];
      assert(JSON.stringify(reads) === JSON.stringify(expected), name + ' dictionary getter order');
      const unsupported = await query({...config, codec: 'not-a-supported-codec'});
      assert(unsupported.supported === false, name + ' unknown codec is unsupported, not invalid');
      const lone = await query({...config, codec: '\ud800'});
      assert(lone.config.codec.charCodeAt(0) === 0xd800, name + ' lossless DOMString');
    }
    const bytes = new Uint8Array([9,1,2,8]);
    const promise = realm.VideoDecoder.isConfigSupported({codec:'vp8', description: new DataView(bytes.buffer, 1, 2),
      get hardwareAcceleration() { bytes[2] = 3; return 'prefer-software'; }, colorSpace: {primaries:'bt709', fullRange:true, ignored:7}});
    bytes.fill(0);
    const cloned = (await promise).config;
    assert(cloned.description instanceof realm.ArrayBuffer && Array.from(new Uint8Array(cloned.description)).join() === '1,3', 'description bytes snapshot after conversion without author access');
    assert(cloned.colorSpace.primaries === 'bt709' && cloned.colorSpace.fullRange === true && cloned.colorSpace.matrix === null && !Object.hasOwn(cloned.colorSpace, 'ignored'), 'nested color-space conversion');
    const buffer = new ArrayBuffer(1);
    let detachedError;
    try { await realm.VideoDecoder.isConfigSupported({codec:'vp8', description:buffer,
      get hardwareAcceleration() { structuredClone(buffer, {transfer:[buffer]}); return 'no-preference'; }}); }
    catch (error) { detachedError = error; }
    assert(detachedError instanceof realm.TypeError, 'description detached by a later getter rejects');
    return 'ok';
  }
  await probe(window);
  await probe(document.getElementById('child').contentWindow);
  const workerSource = `(${probe.toString()})(self).then(value => postMessage(value), error => postMessage(String(error)))`;
  const url = URL.createObjectURL(new Blob([workerSource], {type:'text/javascript'}));
  const result = await new Promise((resolve, reject) => {
    const worker = new Worker(url);
    worker.onmessage = event => { worker.terminate(); resolve(event.data); };
    worker.onerror = event => { event.preventDefault(); worker.terminate(); reject(Error(event.message)); };
  });
  URL.revokeObjectURL(url);
  if (result !== 'ok') throw Error(result);
  return 'ok';
})()
