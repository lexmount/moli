(async () => {
  const checks = [];
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const caught = body => { try { body(); } catch (error) { return error; } };
  const check = async (name, body) => {
    try { await body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error), stack: error.stack}); }
  };
  const child = document.querySelector('iframe').contentWindow;
  const readonly = ['label', 'ordered', 'maxPacketLifeTime', 'maxRetransmits', 'protocol', 'negotiated', 'id', 'readyState', 'bufferedAmount'];
  for (const [realm, w] of [['main', window], ['iframe', child]]) {
    const run = (name, body) => check(`${realm}: ${name}`, async () => {
      const pc = new w.RTCPeerConnection();
      try { await body(pc); } finally { pc.close(); }
    });
    const rejects = (body, name = 'TypeError') => {
      const error = caught(body);
      assert(error instanceof (name === 'TypeError' ? w.TypeError : w.DOMException) && error.name === name, `${name}: ${error}`);
      return error;
    };
    await run('channel has native EventTarget identity and illegal constructor', pc => {
      const c = pc.createDataChannel('one');
      assert(c instanceof w.RTCDataChannel && c instanceof w.EventTarget);
      assert(w.Object.getPrototypeOf(c) === w.RTCDataChannel.prototype);
      assert(w.Object.getPrototypeOf(w.RTCDataChannel.prototype) === w.EventTarget.prototype);
      assert(w.Object.prototype.toString.call(c) === '[object RTCDataChannel]');
      rejects(() => new w.RTCDataChannel()); rejects(() => w.RTCDataChannel());
    });
    await run('channel defaults reflect initialization slots', pc => {
      const c = pc.createDataChannel('');
      assert(c.label === '' && c.protocol === '' && c.ordered === true && c.negotiated === false);
      assert(c.id === null && c.maxPacketLifeTime === null && c.maxRetransmits === null);
      assert(c.readyState === 'connecting' && c.bufferedAmount === 0 && c.bufferedAmountLowThreshold === 0 && c.binaryType === 'arraybuffer');
    });
    await run('readonly attributes and methods have WebIDL descriptors', pc => {
      const c = pc.createDataChannel('');
      for (const name of readonly) {
        const d = w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype, name);
        assert(typeof d.get === 'function' && d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable, name);
        assert(!w.Object.hasOwn(c, name) && w.Reflect.set(c, name, 123) === false);
      }
      for (const name of ['binaryType', 'bufferedAmountLowThreshold']) {
        const d = w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype, name);
        assert(d.get.length === 0 && d.set.length === 1 && d.enumerable && d.configurable && !w.Object.hasOwn(c, name));
      }
      assert(w.RTCPeerConnection.prototype.createDataChannel.length === 1);
      assert(w.RTCDataChannel.prototype.send.length === 1 && w.RTCDataChannel.prototype.close.length === 0);
    });
    await run('label is required but explicit undefined and null are strings', pc => {
      rejects(() => pc.createDataChannel());
      assert(pc.createDataChannel(undefined).label === 'undefined');
      assert(pc.createDataChannel(null).label === 'null');
      assert(pc.createDataChannel(54n).label === '54');
      rejects(() => pc.createDataChannel(Symbol()));
    });
    await run('label and protocol replace lone UTF16 surrogates', pc => {
      const c = pc.createDataChannel('x\ud800y\udc00z\ud83d\ude42', {protocol:'\udc00\ud800'});
      assert(c.label === 'x\ufffdy\ufffdz\ud83d\ude42' && c.protocol === '\ufffd\ufffd');
    });
    await run('maximum label and protocol count UTF8 bytes', pc => {
      for (const value of ['a'.repeat(65535), '\u00e9'.repeat(32767) + 'a']) {
        assert(pc.createDataChannel(value).label === value);
        assert(pc.createDataChannel('', {protocol:value}).protocol === value);
      }
      for (const value of ['a'.repeat(65536), '\u00e9'.repeat(32768), '\ud800'.repeat(21846)]) {
        rejects(() => pc.createDataChannel(value));
        rejects(() => pc.createDataChannel('', {protocol:value}));
      }
    });
    await run('null and undefined dictionary values use WebIDL defaults', pc => {
      for (const options of [undefined, null, {}, {ordered:undefined, negotiated:undefined, protocol:undefined, id:undefined, maxRetransmits:undefined}]) {
        const c = pc.createDataChannel('', options);
        assert(c.ordered && !c.negotiated && c.protocol === '' && c.id === null && c.maxRetransmits === null);
      }
      const c = pc.createDataChannel('', {ordered:null, protocol:null, maxRetransmits:null});
      assert(c.ordered === false && c.protocol === 'null' && c.maxRetransmits === 0);
      for (const options of [1, 'options', true, Symbol(), 1n]) rejects(() => pc.createDataChannel('', options));
    });
    await run('dictionary getters and conversions run once in IDL order', pc => {
      const trace = [], options = {};
      for (const [name, value] of [['protocol', {toString(){trace.push('protocol convert');return 'p';}}], ['ordered', false], ['negotiated', true], ['maxRetransmits', undefined], ['maxPacketLifeTime', 5], ['id', {valueOf(){trace.push('id convert');return 7;}}]])
        w.Object.defineProperty(options, name, {get(){trace.push(name);return value;}});
      const c = pc.createDataChannel({toString(){trace.push('label');return 'l';}}, options);
      assert(trace.join('|') === 'label|id|id convert|maxPacketLifeTime|maxRetransmits|negotiated|ordered|protocol|protocol convert', trace.join('|'));
      assert(c.id === 7 && c.negotiated && c.ordered === false && c.maxPacketLifeTime === 5 && c.protocol === 'p');
    });
    await run('dictionary getter and conversion exception identities survive', pc => {
      const sentinel = {};
      for (const options of [{get id(){throw sentinel;}}, {id:{valueOf(){throw sentinel;}}}, {protocol:{toString(){throw sentinel;}}}])
        assert(caught(() => pc.createDataChannel('', options)) === sentinel);
      assert(caught(() => pc.createDataChannel({toString(){throw sentinel;}})) === sentinel);
    });
    await run('numeric options truncate and accept the unsigned short range', pc => {
      for (const name of ['maxPacketLifeTime', 'maxRetransmits']) {
        for (const [value, expected] of [[0,0],[-0.75,0],[3.9,3],[65535.9,65535],['100',100],[null,0],[true,1]])
          assert(pc.createDataChannel('', {[name]:value})[name] === expected);
        for (const value of [-1,65536,Infinity,-Infinity,NaN,'65536',Symbol(),1n])
          rejects(() => pc.createDataChannel('', {[name]:value}));
      }
    });
    await run('retransmit and lifetime options are mutually exclusive even at zero', pc => {
      for (const [a,b] of [[0,0],[1,2],[null,null]]) rejects(() => pc.createDataChannel('', {maxPacketLifeTime:a,maxRetransmits:b}));
      assert(pc.createDataChannel('', {maxPacketLifeTime:0,maxRetransmits:undefined}).maxPacketLifeTime === 0);
    });
    await run('unnegotiated ids are converted then ignored', pc => {
      for (const id of [0,1,65534,65535,null]) assert(pc.createDataChannel('', {id}).id === null);
      for (const id of [-1,65536,Infinity,NaN,Symbol(),1n]) rejects(() => pc.createDataChannel('', {id}));
    });
    await run('negotiated ids require a usable explicit value', pc => {
      rejects(() => pc.createDataChannel('', {negotiated:true}));
      rejects(() => pc.createDataChannel('', {negotiated:true,id:undefined}));
      rejects(() => pc.createDataChannel('', {negotiated:true,id:65535}));
      for (const id of [0,1,65534]) {
        const c = pc.createDataChannel('', {negotiated:true,id});
        assert(c.negotiated && c.id === id);
      }
    });
    await run('duplicate negotiated ids throw OperationError without changing the original', pc => {
      const c = pc.createDataChannel('original', {negotiated:true,id:45});
      rejects(() => pc.createDataChannel('duplicate', {negotiated:true,id:45}), 'OperationError');
      assert(c.id === 45 && c.label === 'original' && c.readyState === 'connecting');
      const other = new w.RTCPeerConnection();
      try { assert(other.createDataChannel('different pc', {negotiated:true,id:45}).id === 45); } finally { other.close(); }
    });
    await run('duplicate labels create independent channel state', pc => {
      const a = pc.createDataChannel('same'), b = pc.createDataChannel('same');
      a.binaryType = 'blob'; a.bufferedAmountLowThreshold = 27;
      assert(a !== b && b.binaryType === 'arraybuffer' && b.bufferedAmountLowThreshold === 0);
    });
    await run('binaryType accepts both values and ignores out of enum assignments', pc => {
      const c = pc.createDataChannel('');
      for (const value of ['blob','arraybuffer']) { c.binaryType = value; assert(c.binaryType === value); }
      c.binaryType = 'blob';
      for (const value of ['jellyfish','arraybuffer ','',null,undefined,234,54n]) { c.binaryType = value; assert(c.binaryType === 'blob'); }
      rejects(() => {c.binaryType = Symbol();}); assert(c.binaryType === 'blob');
      w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype, 'binaryType').set.call(c);
      assert(c.binaryType === 'blob');
    });
    await run('binaryType conversion preserves side effects and thrown values', pc => {
      const c = pc.createDataChannel(''); let count = 0;
      c.binaryType = {toString(){count++;return 'blob';}}; assert(c.binaryType === 'blob' && count === 1);
      const sentinel = {}; assert(caught(() => {c.binaryType = {toString(){throw sentinel;}};}) === sentinel);
      assert(c.binaryType === 'blob');
    });
    await run('threshold uses EnforceRange unsigned long before mutation', pc => {
      const c = pc.createDataChannel('');
      for (const [value, expected] of [[0,0],[-0.75,0],[5.9,5],[4294967295.9,4294967295],['12',12],[null,0]]) {
        c.bufferedAmountLowThreshold = value; assert(c.bufferedAmountLowThreshold === expected);
      }
      c.bufferedAmountLowThreshold = 8;
      for (const value of [-1,4294967296,Infinity,-Infinity,NaN,undefined,Symbol(),1n]) {
        rejects(() => {c.bufferedAmountLowThreshold = value;}); assert(c.bufferedAmountLowThreshold === 8);
      }
      let count = 0; c.bufferedAmountLowThreshold = {valueOf(){count++;return 19;}};
      assert(count === 1 && c.bufferedAmountLowThreshold === 19);
      const sentinel = {}; assert(caught(() => {c.bufferedAmountLowThreshold = {valueOf(){throw sentinel;}};}) === sentinel);
      assert(c.bufferedAmountLowThreshold === 19);
    });
    await run('closed PC still converts all arguments before InvalidStateError', pc => {
      pc.close(); const trace = [];
      rejects(() => pc.createDataChannel({toString(){trace.push('label');return ''; }}, {get id(){trace.push('id');return undefined;},get protocol(){trace.push('protocol');return '';}}), 'InvalidStateError');
      assert(trace.join('|') === 'label|id|protocol');
      const sentinel = {}; assert(caught(() => pc.createDataChannel('', {get id(){throw sentinel;}})) === sentinel);
      rejects(() => pc.createDataChannel('', {maxRetransmits:65536}));
      rejects(() => pc.createDataChannel('a'.repeat(65536)), 'InvalidStateError');
    });
    await run('invalid receivers fail before argument conversion or author traps', pc => {
      const c = pc.createDataChannel(''), revoked = w.Proxy.revocable(c, {}); revoked.revoke();
      let traps = 0, conversions = 0;
      const fake = new w.Proxy(c, {get(){traps++;throw Error('trap');},getPrototypeOf(){traps++;throw Error('trap');}});
      const value = {toString(){conversions++;return 'blob';},valueOf(){conversions++;return 3;}};
      for (const receiver of [{},w.Object.create(w.RTCDataChannel.prototype),w.Object.create(c),fake,revoked.proxy,null,undefined]) {
        for (const name of readonly) rejects(() => w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype, name).get.call(receiver));
        for (const name of ['binaryType','bufferedAmountLowThreshold']) rejects(() => w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype, name).set.call(receiver,value));
        rejects(() => w.RTCDataChannel.prototype.close.call(receiver));
        rejects(() => w.RTCDataChannel.prototype.send.call(receiver,value));
      }
      const proxyPC = new w.Proxy(pc, {get(){traps++;throw Error('trap');}});
      rejects(() => w.RTCPeerConnection.prototype.createDataChannel.call(proxyPC,value,{get id(){conversions++;return 5;}}));
      assert(traps === 0 && conversions === 0);
    });
    await run('native channel construction ignores overwritten globals and array prototype setters', pc => {
      const Original = w.RTCDataChannel;
      const descriptor = w.Object.getOwnPropertyDescriptor(w.Array.prototype, '0'); let reads = 0;
      try {
        w.Object.defineProperty(w, 'RTCDataChannel', {configurable:true,get(){reads++;throw Error('global constructor');}});
        w.Object.defineProperty(w.Array.prototype, '0', {configurable:true,set(){throw Error('array setter');}});
        const c = pc.createDataChannel('safe');
        assert(w.Object.getPrototypeOf(c) === Original.prototype && c.label === 'safe' && reads === 0);
      } finally {
        w.Object.defineProperty(w, 'RTCDataChannel', {value:Original,configurable:true,writable:true});
        if (descriptor) w.Object.defineProperty(w.Array.prototype, '0', descriptor); else delete w.Array.prototype[0];
      }
    });
    await run('first data channel triggers one trusted asynchronous negotiation event', async pc => {
      const trace = []; pc.onnegotiationneeded = function(e){ assert(this === pc && e.target === pc && e.isTrusted && !e.bubbles && !e.cancelable); trace.push('event'); };
      pc.createDataChannel('a'); pc.createDataChannel('b');
      assert(trace.length === 0); await Promise.resolve(); assert(trace.length === 0);
      await new Promise(resolve => setTimeout(resolve, 100)); assert(trace.join('|') === 'event', trace.join('|'));
    });
    await run('failed creations do not trigger negotiation needed', async pc => {
      let count = 0; pc.onnegotiationneeded = () => count++;
      rejects(() => pc.createDataChannel('', {negotiated:true}));
      rejects(() => pc.createDataChannel('', {maxPacketLifeTime:0,maxRetransmits:0}));
      rejects(() => pc.createDataChannel('a'.repeat(65536)));
      await new Promise(resolve => setTimeout(resolve, 50)); assert(count === 0);
    });
    await run('closing before the networking task suppresses negotiation event', async pc => {
      let count = 0; pc.onnegotiationneeded = () => count++;
      pc.createDataChannel(''); pc.close(); await new Promise(resolve => setTimeout(resolve, 50)); assert(count === 0);
    });
  }
  for (const [name, callee, receiverRealm] of [['child callee',child,window],['main callee',window,child]]) {
    await check(`${name}: borrowed creation returns a channel in the callee realm`, () => {
      const pc = new receiverRealm.RTCPeerConnection();
      try {
        const c = callee.RTCPeerConnection.prototype.createDataChannel.call(pc, 'foreign', {negotiated:true,id:93});
        assert(callee.Object.getPrototypeOf(c) === callee.RTCDataChannel.prototype && c.label === 'foreign' && c.id === 93);
      } finally { pc.close(); }
    });
    await check(`${name}: borrowed channel getters setters and errors use native brands`, () => {
      const pc = new receiverRealm.RTCPeerConnection(), c = pc.createDataChannel('foreign');
      try {
        const p = callee.RTCDataChannel.prototype;
        assert(callee.Object.getOwnPropertyDescriptor(p,'label').get.call(c) === 'foreign');
        callee.Object.getOwnPropertyDescriptor(p,'binaryType').set.call(c,'blob'); assert(c.binaryType === 'blob');
        const setter = callee.Object.getOwnPropertyDescriptor(p,'bufferedAmountLowThreshold').set;
        setter.call(c,51); assert(c.bufferedAmountLowThreshold === 51);
        for (const body of [() => setter.call(c,-1), () => setter.call({},0), () => p.close.call(new Proxy(c,{}))]) {
          const error = caught(body); assert(error instanceof callee.TypeError && !(error instanceof receiverRealm.TypeError));
        }
      } finally { pc.close(); }
    });
  }
  globalThis.__uiEventResults = {complete:true,passed:checks.filter(c=>c.passed).length,total:checks.length,checks};
  return checks.every(c=>c.passed);
})()
