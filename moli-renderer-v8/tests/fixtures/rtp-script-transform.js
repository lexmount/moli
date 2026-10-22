(async () => {
  const checks = [];
  globalThis.__codecChecks = checks;
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const equal = (actual, expected) => assert(JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify({actual, expected}));
  const raises = (action, C, name) => {
    let error; try { action(); } catch (caught) { error = caught; }
    assert(error instanceof C && (!name || error.name === name), 'expected '+(name || C.name)+', got '+error);
  };
  function workerProgram() {
    const exposure = [typeof RTCRtpScriptTransformer, typeof RTCTransformEvent, typeof KeyFrameRequestEvent, typeof RTCRtpScriptTransform, 'onrtctransform' in self, Function.prototype.toString.call(KeyFrameRequestEvent)];
    const held = new Map();
    const assert = ok => { if (!ok) throw Error('worker assertion'); };
    const rejectName = async (action, C, name) => {
      const promise = action(); assert(promise instanceof Promise);
      let error; try { await promise; } catch (caught) { error = caught; }
      assert(error instanceof C && (!name || error.name === name));
    };
    self.onrtctransform = event => {
      const t = event.transformer, options = t.options, key = options.key;
      (async () => {
        const facts = [];
        const check = (name, value) => facts.push({name, passed: !!value});
        check('trusted native event', event instanceof RTCTransformEvent && event instanceof Event && event.isTrusted && event.target === self && event.currentTarget === self);
        check('native transformer EventTarget', t instanceof RTCRtpScriptTransformer && t instanceof EventTarget);
        check('real Streams and stable options', t.readable instanceof ReadableStream && t.writable instanceof WritableStream && t.options === options);
        const writer = t.writable.getWriter();
        check('infinite high water mark', writer.desiredSize === Infinity);
        const get = Object.getOwnPropertyDescriptor(RTCRtpScriptTransformer.prototype, 'options').get;
        let rejected = 0, traps = 0, conversions = 0;
        const revoked = Proxy.revocable(t, {}); revoked.revoke();
        for (const bad of [{}, Object.create(t), new Proxy(t, {get(){traps++;}}), revoked.proxy]) {
          try { get.call(bad); } catch (e) { if (e instanceof TypeError) rejected++; }
          await rejectName(() => RTCRtpScriptTransformer.prototype.generateKeyFrame.call(bad, {toString(){conversions++;return 'x';}}), TypeError);
        }
        check('native brands before conversion', rejected === 4 && traps === 0 && conversions === 0);
        await rejectName(() => t.generateKeyFrame(), DOMException, 'InvalidStateError');
        await rejectName(() => t.sendKeyFrameRequest(), DOMException, 'InvalidStateError');
        check('no fabricated frame source', true);
        const marker = {};
        try { await t.generateKeyFrame({toString(){throw marker;}}); } catch (e) { check('RID exception identity', e === marker); }
        const calls = [];
        t.addEventListener('keyframerequest', () => calls.push('listener'));
        t.onkeyframerequest = e => { assert(e.rid === '\ud800'); calls.push('handler'); };
        t.dispatchEvent(new KeyFrameRequestEvent('keyframerequest', '\ud800'));
        check('ordered keyframe event handler', calls.join(',') === 'listener,handler');
        if (options.mode === 'bad-chunk') {
          await rejectName(() => writer.write({}), TypeError);
          writer.closed.then(() => {}, () => {});
          check('invalid chunk rejects', true);
        } else if (options.mode === 'hold') {
          const reader = t.readable.getReader();
          held.set(key, Promise.all([
            reader.read().then(r => r.done && r.value === undefined),
            reader.closed.then(() => true),
            writer.closed.then(() => false, error => error === undefined)
          ]).then(values => values.every(Boolean) && t.readable.locked && t.writable.locked));
        } else writer.releaseLock();
        if (options.port) {
          options.port.onmessage = e => options.port.postMessage(e.data + 1);
        }
        postMessage({kind:'event', key, facts, snapshot:options.snapshot, bytes:options.bytes ? Array.from(new Uint8Array(options.bytes)) : null});
      })().catch(error => postMessage({kind:'event', key, error:String(error)}));
    };
    self.onmessage = async event => {
      const {key, command} = event.data;
      if (command === 'closed') postMessage({kind:'closed', key, closed:await held.get(key)});
      else postMessage({kind:'ping', key});
    };
    postMessage({kind:'ready', key:'ready', exposed:exposure});
  }
  for (const [index, w] of [window, document.querySelector('iframe').contentWindow].entries()) {
    for (const kind of ['classic', 'module']) {
      const prefix = index+':'+kind+':';
      const check = async (name, action) => {
        globalThis.__codecCurrentCheck = prefix+name;
        try { await action(); checks.push({name:prefix+name, passed:true}); }
        catch (error) { checks.push({name:prefix+name, passed:false, error:{name:error.name, message:error.message}}); }
      };
      const workerURL = w.URL.createObjectURL(new w.Blob([
        (kind === 'module' ? 'await new Promise(resolve => setTimeout(resolve, 0));\n' : '')+'('+workerProgram.toString()+')();'
      ], {type:'text/javascript'}));
      const worker = new w.Worker(workerURL, {type:kind});
      const waiting = new Map(), received = new Map();
      worker.onmessage = event => {
        const key = event.data.kind+':'+event.data.key;
        received.set(key, event.data);
        const entry=waiting.get(key);
        if(entry) {clearTimeout(entry.timer);entry.resolve(event.data);waiting.delete(key);}
      };
      const wait = key => received.has(key) ? Promise.resolve(received.get(key)) : new Promise((resolve,reject) => {
        const timer=setTimeout(()=>{waiting.delete(key);reject(Error('Worker event not delivered: '+key));},3000);
        waiting.set(key,{resolve,timer});
      });
      const T = w.RTCRtpScriptTransform, pc = new w.RTCPeerConnection();
      let next = 0;
      const create = async (options = {}, transfer = [], Constructor = T) => {
        const key = prefix+(++next);
        const t = new Constructor(worker, {...options, key}, transfer);
        const event = await wait('event:'+key);
        assert(!event.error, event.error);
        for (const row of event.facts) checks.push({name:prefix+'worker:'+key+':'+row.name, passed:row.passed});
        return {t, event, key};
      };
      try {
        // Queue this before either classic or module bootstrap has completed.
        await check('constructor before Worker bootstrap', async () => { await create(); });
        await check('cross-realm Worker uses its own task queue', async () => {
          const other = index === 0 ? document.querySelector('iframe').contentWindow : window;
          await create({}, [], other.RTCRtpScriptTransform);
        });
        await check('Dedicated Worker exposure', async () => equal((await wait('ready:ready')).exposed, ['function','function','function','undefined',true,'function KeyFrameRequestEvent() { [native code] }']));
        await check('Window exposure and arity', () => assert(T.length === 1 && T.name === 'RTCRtpScriptTransform' && !('RTCRtpScriptTransformer' in w) && !('RTCTransformEvent' in w) && !('KeyFrameRequestEvent' in w)));
        await check('requires construct and Worker', () => { raises(() => T(worker), w.TypeError); raises(() => new T(), w.TypeError); });
        await check('rejects forged Worker brands', () => {
          for (const bad of [{}, Object.create(w.Worker.prototype), Object.create(worker), new Proxy(worker,{})]) raises(() => new T(bad), w.TypeError);
        });
        await check('transfer sequence before option cloning', () => {
          let reads=0;
          raises(() => new T(worker, {get option(){reads++;return 1;}}, [7]), w.TypeError);
          assert(reads === 0);
        });
        await check('duplicate transfer is atomic', () => {
          const bytes = new w.ArrayBuffer(2); let reads=0;
          raises(() => new T(worker, {get option(){reads++;return 1;}}, [bytes,bytes]), w.DOMException, 'DataCloneError');
          assert(bytes.byteLength === 2 && reads === 0);
        });
        await check('uncloneable options preserve transfer buffers', () => {
          const bytes = new w.ArrayBuffer(2);
          raises(() => new T(worker, {bytes, bad(){}}, [bytes]), w.DOMException, 'DataCloneError');
          assert(bytes.byteLength === 2);
        });
        await check('clone getter exception preserves identity', () => {
          const marker = {}, bytes = new w.ArrayBuffer(2); let error;
          try { new T(worker, {get option(){throw marker;}}, [bytes]); } catch (e) { error=e; }
          assert(error === marker && bytes.byteLength === 2);
        });
        await check('options snapshot and ArrayBuffer transfer', async () => {
          const bytes = new w.ArrayBuffer(3); new w.Uint8Array(bytes).set([1,2,3]);
          const key = prefix+(++next), nested = {value:7}; let reads=0;
          new T(worker, {key, bytes, get snapshot(){reads++;return nested.value;}}, [bytes]);
          nested.value=9; assert(reads===1 && bytes.byteLength===0);
          const event=await wait('event:'+key); assert(!event.error,event.error); equal(event.bytes,[1,2,3]); assert(event.snapshot===7);
        });
        await check('MessagePort transfer', async () => {
          const channel = new w.MessageChannel();
          try {
            await create({port:channel.port2}, [channel.port2]);
            const answer = new Promise(resolve => channel.port1.onmessage = e => resolve(e.data));
            channel.port1.postMessage(41); assert(await answer === 42);
          } finally { channel.port1.close(); channel.port2.close(); }
        });
        await check('native write rejects bad chunks', async () => { await create({mode:'bad-chunk'}); });
        await check('permanent ownership and same assignment', async () => {
          const first=pc.addTransceiver('audio'), second=pc.addTransceiver('audio'), {t}=await create();
          assert(first.sender.transform===null && first.receiver.transform===null);
          first.sender.transform=t; assert(first.sender.transform===t); first.sender.transform=t;
          raises(() => first.receiver.transform=t, w.DOMException, 'InvalidStateError');
          raises(() => second.sender.transform=t, w.DOMException, 'InvalidStateError');
          first.sender.transform=null; assert(first.sender.transform===null);
          raises(() => first.sender.transform=t, w.DOMException, 'InvalidStateError');
          raises(() => second.sender.transform=t, w.DOMException, 'InvalidStateError');
        });
        for (const endpoint of ['sender','receiver']) await check(endpoint+' disassociation closes locked Streams', async () => {
          const item=pc.addTransceiver('audio')[endpoint], {t,key}=await create({mode:'hold'});
          item.transform=t; item.transform=null;
          worker.postMessage({command:'closed', key}); assert((await wait('closed:'+key)).closed === true);
        });
        await check('receiver rejects author proxies without traps', async () => {
          const {t}=await create(), target=pc.addTransceiver('audio').receiver;
          const setter=Object.getOwnPropertyDescriptor(w.RTCRtpReceiver.prototype,'transform').set;
          let traps=0; const proxy=new Proxy(t,{get(){traps++;}});
          raises(() => setter.call({}, proxy),w.TypeError);
          raises(() => target.transform=proxy,w.TypeError); assert(traps===0 && target.transform===null);
        });
        await check('WorkerAndParameters dictionary member ordering', async () => {
          const log=[], key=prefix+(++next);
          new T({get type(){log.push('type');return undefined;},get worker(){log.push('worker');return worker;}}, {key});
          equal(log,['type','worker']); const event=await wait('event:'+key); assert(!event.error,event.error);
        });
        await check('dictionary invalid type before worker getter', () => {
          let reads=0; raises(() => new T({type:'invalid',get worker(){reads++;return worker;}}),w.TypeError); assert(reads===0);
        });
        await check('native NewTarget prototype precedes conversion and serialization', () => {
          const order=[], marker={}, bytes=new w.ArrayBuffer(1);
          const argument={get type(){order.push('type');return undefined;},get worker(){order.push('worker');return worker;}};
          const options={get option(){order.push('clone');return 1;}};
          const transfer={[Symbol.iterator](){order.push('iterator');let done=false;return {next(){order.push('next');if(done)return {done:true};done=true;return {done:false,value:bytes};}};}};
          const newTarget=new Proxy(function(){},{get(target,key,receiver){if(key==='prototype'){order.push('prototype');throw marker;}return Reflect.get(target,key,receiver);}});
          let error;try {Reflect.construct(T,[argument,options,transfer],newTarget);}catch(e){error=e;}
          assert(error===marker && bytes.byteLength===1);equal(order,['prototype']);
        });
        await check('SFrame mode is fixed by first synchronous assignment', () => {
          const scoped = new w.RTCPeerConnection();
          try {
            const item=scoped.addTransceiver('audio');
            const transform=new T({worker,type:'sframe'}, {key:prefix+'sframe'});
            item.sender.transform=transform; assert(item.sender.transform===transform);
            raises(() => item.receiver.transform=new T(worker,{key:prefix+'plain'}),w.DOMException,'InvalidModificationError');
            raises(() => item.sender.transform=null,w.DOMException,'InvalidModificationError');
          } finally { scoped.close(); }
        });
      } finally { pc.close(); worker.terminate(); w.URL.revokeObjectURL(workerURL); }
    }
  }
  globalThis.__uiEventResults = {complete:true, total:checks.length, passed:checks.filter(row=>row.passed).length, checks};
  return checks.every(row=>row.passed);
})()
