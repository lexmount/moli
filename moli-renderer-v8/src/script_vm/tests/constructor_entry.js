(async () => {
  const rows = [];
  const check = (name, ok, detail = null) => rows.push({name, ok: !!ok, detail});
  const child = document.querySelector('iframe');
  await new Promise(resolve => {
    if (child.contentDocument.readyState === 'complete') resolve();
    else child.addEventListener('load', resolve, {once: true});
  });
  const other = child.contentWindow;
  const definitions = [
    ['DOMParser', () => [], 'DOMParser'],
    ['VideoDecoder', () => [{error() {}, output() {}}], 'VideoDecoder'],
    ['VideoEncoder', () => [{error() {}, output() {}}], 'VideoEncoder'],
    ['VideoFrame', w => [new w.Uint8Array(16), {format: 'RGBA', codedWidth: 2, codedHeight: 2, timestamp: 0}], 'VideoFrame'],
    ['MediaMetadata', () => [{title: 'native'}], 'MediaMetadata'],
    ['MediaStream', () => [], 'MediaStream'],
    ['MediaRecorder', w => [new w.MediaStream()], 'MediaRecorder'],
    ['RTCDTMFToneChangeEvent', () => ['x', {tone: '1'}], 'RTCDTMFToneChangeEvent'],
    ['VideoColorSpace', () => [{matrix: 'bt709'}], 'VideoColorSpace'],
    ['RTCIceCandidate', () => [{candidate: '', sdpMid: '0'}], 'RTCIceCandidate'],
    ['RTCSessionDescription', () => [{type: 'offer', sdp: ''}], 'RTCSessionDescription'],
    ['RTCError', () => [{errorDetail: 'sdp-syntax-error'}, 'reason'], 'RTCError'],
    ['RTCDataChannelEvent', w => ['x', {channel: channels.get(w)}], 'RTCDataChannelEvent'],
    ['RTCErrorEvent', w => ['x', {error: new w.RTCError({errorDetail: 'sdp-syntax-error'})}], 'RTCErrorEvent'],
    ['RTCPeerConnectionIceEvent', () => ['x', {candidate: null}], 'RTCPeerConnectionIceEvent'],
    ['RTCPeerConnectionIceErrorEvent', () => ['x', {errorCode: 701}], 'RTCPeerConnectionIceErrorEvent'],
    ['Image', () => [1, 2], 'HTMLImageElement'],
    ['Audio', () => [''], 'HTMLAudioElement'],
    ['Option', () => ['label', 'value', true, true], 'HTMLOptionElement']
  ];
  const connections = [];
  const channels = new Map();
  try {
    for (const w of [window, other]) {
      const realm = w === window ? 'main' : 'child';
      const peer = new w.RTCPeerConnection(); connections.push(peer);
      channels.set(w, peer.createDataChannel('constructor-test')); 
      for (const [name, args, defaultName] of definitions) {
        try {
          const C = w[name], defaults = w[defaultName].prototype;
          check(`${realm}/${name}/metadata`, typeof C === 'function' && C.name === name && C.prototype === defaults);
          check(`${realm}/${name}/native-source`, Function.prototype.toString.call(C) === `function ${name}() { [native code] }`);
          class Derived extends C {}
          const derived = new Derived(...args(w));
          check(`${realm}/${name}/subclass`, Object.getPrototypeOf(derived) === Derived.prototype && derived instanceof Derived && derived instanceof C);
          const explicit = [w.Object.prototype, Object.create(null), new Proxy({}, {})];
          for (const [i, prototype] of explicit.entries()) {
            const N = new Proxy(w.Function(), {get(target, key, receiver) {
              if (key === 'prototype') return prototype;
              return Reflect.get(target, key, receiver);
            }});
            const value = Reflect.construct(C, args(w), N);
            check(`${realm}/${name}/explicit-${i}`, Object.getPrototypeOf(value) === prototype);
          }
          for (const nw of [window, other]) {
            let reads = 0;
            const N = new Proxy(nw.Function(), {get(target, key, receiver) {
              if (key === 'prototype') { reads++; return 42; }
              return Reflect.get(target, key, receiver);
            }});
            const value = Reflect.construct(C, args(w), N);
            check(`${realm}/${name}/fallback-${nw === window ? 'main' : 'child'}`, Object.getPrototypeOf(value) === nw.Object.prototype && reads === 1, {reads});
          }
          const prototype = Object.create(null);
          let reads = 0, nested = null;
          const N = new Proxy(w.Function(), {get(target, key, receiver) {
            if (key === 'prototype') {
              reads++;
              if (reads === 1) nested = Reflect.construct(C, args(w), N);
              return prototype;
            }
            return Reflect.get(target, key, receiver);
          }});
          const outer = Reflect.construct(C, args(w), N);
          check(`${realm}/${name}/reentry`, reads === 2 && Object.getPrototypeOf(outer) === prototype && Object.getPrototypeOf(nested) === prototype, {reads});
          const sentinel = {};
          const throwing = new Proxy(w.Function(), {get(target, key, receiver) {
            if (key === 'prototype') throw sentinel;
            return Reflect.get(target, key, receiver);
          }});
          let thrown;
          try { Reflect.construct(C, args(w), throwing); } catch (error) { thrown = error; }
          check(`${realm}/${name}/prototype-exception`, thrown === sentinel);
        } catch (error) { check(`${realm}/${name}/setup`, false, String(error)); }
      }
      for (const kind of ['object', 'primitive']) {
        let revoke, reads = 0;
        const prototype = Object.create(null);
        const pair = Proxy.revocable(w.Function(), {get(target, key, receiver) {
          if (key === 'prototype') { reads++; revoke(); return kind === 'object' ? prototype : 7; }
          return Reflect.get(target, key, receiver);
        }});
        revoke = pair.revoke;
        let value, error;
        try { value = Reflect.construct(w.VideoColorSpace, [], pair.proxy); } catch (e) { error = e; }
        check(`${realm}/revoked-after-get-${kind}`, reads === 1 && (kind === 'object' ? value && Object.getPrototypeOf(value) === prototype : error instanceof w.TypeError), {reads, error: String(error)});
      }
      for (const [name, member, init] of [
        ['RTCIceCandidate', 'candidate', {sdpMid: '0'}],
        ['RTCSessionDescription', 'sdp', {type: 'offer'}]
      ]) {
        const C = w[name], N = w.Function();
        const oldPrototype = Object.create(C.prototype), newPrototype = Object.create(C.prototype);
        N.prototype = oldPrototype;
        Object.defineProperty(init, member, {get() { N.prototype = newPrototype; return ''; }});
        const value = Reflect.construct(C, [init], N);
        check(`${realm}/${name}/conversion-keeps-selected-prototype`,
          Object.getPrototypeOf(value) === oldPrototype && N.prototype === newPrototype);
      }
      {
        const log = [], marker = {}, bytes = new w.Uint8Array(16);
        const init = {format: 'RGBA', codedWidth: 2, codedHeight: 2,
          get timestamp() { log.push('conversion'); return 0; }, transfer: [bytes.buffer]};
        const N = new Proxy(w.Function(), {get(target, key, receiver) {
          if (key === 'prototype') { log.push('prototype'); throw marker; }
          return Reflect.get(target, key, receiver);
        }});
        let caught;
        try { Reflect.construct(w.VideoFrame, [bytes, init], N); } catch (error) { caught = error; }
        check(`${realm}/VideoFrame/prototype-exception-prevents-conversion-and-transfer`,
          caught === marker && log.join() === 'prototype' && bytes.byteLength === 16, [...log]);
      }
      const log = [];
      const N = new Proxy(w.Function(), {get(target, key, receiver) {
        if (key === 'prototype') { log.push('prototype'); return w.Object.prototype; }
        return Reflect.get(target, key, receiver);
      }});
      const init = {get fullRange() { log.push('fullRange'); return true; }, get matrix() { log.push('matrix'); return 'bt709'; }, get primaries() { log.push('primaries'); return 'bt709'; }, get transfer() { log.push('transfer'); return 'bt709'; }};
      Reflect.construct(w.VideoColorSpace, [init], N);
      check(`${realm}/native-prototype-before-conversion`, log.join(',') === 'prototype,fullRange,matrix,primaries,transfer', [...log]);
      log.length = 0;
      const sentinel = {};
      try { Reflect.construct(w.VideoColorSpace, [{get matrix() { log.push('matrix'); throw sentinel; }}], N); } catch (error) { check(`${realm}/conversion-exception-identity`, error === sentinel); }
      check(`${realm}/failed-conversion-retains-prototype-read`, log.join(',') === 'prototype,matrix', [...log]);
    }
    // Native construction does not consult an author Reflect.construct or
    // Object.prototype trap names, including during lazy materialization.
    const intrinsicConstruct = Reflect.construct;
    const prototypeGet = Object.prototype.get;
    const inheritedConstruct = Object.prototype.construct;
    try {
      Reflect.construct = () => { throw Error('author Reflect.construct'); };
      Object.prototype.get = () => { throw Error('inherited get trap'); };
      Object.prototype.construct = () => { throw Error('inherited construct trap'); };
      check('native-construction-ignores-author-reflection', Object.getPrototypeOf(new VideoColorSpace()) === VideoColorSpace.prototype);
    } finally {
      Reflect.construct = intrinsicConstruct;
      if (prototypeGet === undefined) delete Object.prototype.get; else Object.prototype.get = prototypeGet;
      if (inheritedConstruct === undefined) delete Object.prototype.construct; else Object.prototype.construct = inheritedConstruct;
    }
  } finally { for (const connection of connections) connection.close(); }
  globalThis.__uiEventResults = {rows, total: rows.length, passed: rows.filter(row => row.ok).length, failed: rows.filter(row => !row.ok)};
  return true;
})()
