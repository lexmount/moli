(async () => {
  const checks = [];
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const caught = body => { try { body(); } catch (error) { return error; } };
  const check = async (name, body) => {
    try { await body(); checks.push({name, passed:true}); }
    catch (error) { checks.push({name, passed:false, error:String(error), stack:error.stack}); }
  };
  const child = document.querySelector('iframe').contentWindow;
  for (const [realm,w] of [['main',window],['iframe',child]]) {
    const run = (name,body) => check(`${realm}: ${name}`, () => {
      const pc = new w.RTCPeerConnection(), channel = pc.createDataChannel('send');
      try { body(channel,pc); } finally { pc.close(); }
    });
    const rejects = (body,name='InvalidStateError') => {
      const error = caught(body);
      assert(error instanceof (name==='TypeError'?w.TypeError:w.DOMException) && error.name===name, `${name}: ${error}`);
      if(name==='InvalidStateError') assert(error.code===11);
      return error;
    };
    await run('send descriptor and required arity', c => {
      const d = w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype,'send');
      assert(d.value.name==='send' && d.value.length===1 && d.enumerable && d.configurable && d.writable && !w.Object.hasOwn(c,'send'));
      rejects(()=>c.send(),'TypeError'); assert(c.bufferedAmount===0 && c.readyState==='connecting');
    });
    for (const [name,value] of [['empty string',''],['ASCII string','hello'],['unicode string','世界你好'],['lone surrogate string','\ud800x\udc00'],['undefined',undefined],['null',null],['boolean',true],['number',51],['NaN',NaN],['Infinity',Infinity],['BigInt',17n]])
      await run(`connecting rejects ${name} after string conversion`, c => { rejects(()=>c.send(value)); assert(c.bufferedAmount===0); });
    await run('Symbol fails string conversion before readyState', c => { rejects(()=>c.send(Symbol()),'TypeError'); assert(c.bufferedAmount===0); });
    await run('ordinary object conversion runs once with string hint', c => {
      let count=0;const value={[Symbol.toPrimitive](hint){count++;assert(hint==='string');return 'message';}};
      rejects(()=>c.send(value));assert(count===1 && c.bufferedAmount===0);
    });
    await run('ordinary object throwing conversion keeps exact exception', c => {
      const sentinel={}; assert(caught(()=>c.send({toString(){throw sentinel;}}))===sentinel);
      assert(c.bufferedAmount===0 && c.readyState==='connecting');
    });
    await run('conversion observes reentrant close before state check', c => {
      rejects(()=>c.send({toString(){c.close();return 'message';}}));
      assert(c.readyState!=='open' && c.bufferedAmount===0);
    });
    await run('extra arguments are ignored without conversion', c => {
      let count=0;rejects(()=>c.send('message',{toString(){count++;throw Error('extra');}}));assert(count===0);
    });
    await run('Blob and File select interface overload without author getters', c => {
      for(const value of [new w.Blob(['abc']),new w.File(['abc'],'f.txt')]) {
        let reads=0;
        for(const key of ['size','type','arrayBuffer','stream','text','toString',Symbol.toPrimitive,Symbol.toStringTag])
          w.Object.defineProperty(value,key,{get(){reads++;throw Error('blob getter');},configurable:true});
        rejects(()=>c.send(value));assert(reads===0 && c.bufferedAmount===0);
      }
    });
    await run('ArrayBuffer and all fixed views avoid author properties', c => {
      const values=[new w.ArrayBuffer(4),new w.DataView(new w.ArrayBuffer(8),2,4)];
      for(const name of ['Int8Array','Uint8Array','Uint8ClampedArray','Int16Array','Uint16Array','Int32Array','Uint32Array','Float32Array','Float64Array','BigInt64Array','BigUint64Array'])values.push(new w[name](4));
      for(const value of values) {
        let reads=0;
        for(const key of ['byteLength','byteOffset','buffer','resizable','toString',Symbol.toPrimitive,Symbol.toStringTag])
          w.Object.defineProperty(value,key,{get(){reads++;throw Error('buffer getter');},configurable:true});
        rejects(()=>c.send(value));assert(reads===0 && c.bufferedAmount===0);
      }
    });
    await run('detached fixed buffers and views reach the state check', c => {
      const buffer=new w.ArrayBuffer(8),typed=new w.Uint8Array(buffer),view=new w.DataView(buffer);
      w.structuredClone(buffer,{transfer:[buffer]});
      for(const value of [buffer,typed,view])rejects(()=>c.send(value));
      assert(c.bufferedAmount===0);
    });
    await run('resizable buffers and their fixed and tracking views reject before state', c => {
      const b=new w.ArrayBuffer(8,{maxByteLength:16});
      for(const value of [b,new w.Uint8Array(b),new w.Uint8Array(b,0,4),new w.DataView(b),new w.DataView(b,0,4)]) {
        w.Object.defineProperty(value,'resizable',{value:false});rejects(()=>c.send(value),'TypeError');
      }
      assert(c.bufferedAmount===0);
    });
    await run('out of bounds resizable views reject before state', c => {
      const b=new w.ArrayBuffer(8,{maxByteLength:16}),typed=new w.Uint8Array(b,4,4),view=new w.DataView(b,4,4);
      b.resize(0);for(const value of [typed,view])rejects(()=>c.send(value),'TypeError');
    });
    await run('detached resizable buffers retain conversion rejection', c => {
      const b=new w.ArrayBuffer(8,{maxByteLength:16}),typed=new w.Uint8Array(b),view=new w.DataView(b);
      w.structuredClone(b,{transfer:[b]});for(const value of [b,typed,view])rejects(()=>c.send(value),'TypeError');
    });
    await run('binary overload has priority over custom stringification', c => {
      const b=new w.ArrayBuffer(2),view=new w.Uint8Array(b),blob=new w.Blob(['x']);let count=0;
      for(const value of [b,view,blob]) {
        value[Symbol.toPrimitive]=()=>{count++;throw Error('stringifier');};rejects(()=>c.send(value));
      }
      assert(count===0);
    });
    await run('author Proxy data arguments use ordinary string overload without unwrapping', c => {
      for(const target of [new w.Blob(['x']),new w.ArrayBuffer(2),new w.Uint8Array(2)]) {
        let reads=0,calls=0;
        const value=new w.Proxy(target,{get(target,key){reads++;assert(key===Symbol.toPrimitive);return hint=>{calls++;assert(hint==='string');return 'proxy';};}});
        rejects(()=>c.send(value));assert(reads===1 && calls===1);
      }
    });
    await run('forged Blob and ArrayBuffer prototypes still use string overload', c => {
      for(const prototype of [w.Blob.prototype,w.ArrayBuffer.prototype,w.Uint8Array.prototype]) {
        const value=w.Object.create(prototype);let count=0;
        value[Symbol.toPrimitive]=hint=>{assert(hint==='string');count++;return 'forged';};
        rejects(()=>c.send(value));assert(count===1);
      }
    });
    await run('revoked Proxy data argument conversion throws TypeError', c => {
      const value=w.Proxy.revocable(new w.Blob(),{});value.revoke();rejects(()=>c.send(value.proxy),'TypeError');
    });
    await run('invalid receiver wins over conversions and author traps', c => {
      let reads=0,conversions=0;const value={toString(){conversions++;return 'message';}};
      const proxy=new w.Proxy(c,{get(){reads++;throw Error('receiver trap');},getPrototypeOf(){reads++;throw Error('receiver trap');}});
      const revoked=w.Proxy.revocable(c,{});revoked.revoke();
      for(const receiver of [{},w.Object.create(w.RTCDataChannel.prototype),w.Object.create(c),proxy,revoked.proxy,null,undefined]) {
        rejects(()=>w.RTCDataChannel.prototype.send.call(receiver,value),'TypeError');
        rejects(()=>w.RTCDataChannel.prototype.send.call(receiver),'TypeError');
      }
      assert(reads===0 && conversions===0);
    });
    await run('closed channel performs conversion and rejects without buffering', c => {
      c.close();let count=0;rejects(()=>c.send({toString(){count++;return 'x';}}));
      assert(count===1 && c.bufferedAmount===0 && c.readyState!=='open');
      const sentinel={};assert(caught(()=>c.send({toString(){throw sentinel;}}))===sentinel);
      rejects(()=>c.send(Symbol()),'TypeError');rejects(()=>c.send(),'TypeError');
    });
    await run('spoofed public readyState does not bypass native state', c => {
      w.Object.defineProperty(c,'readyState',{value:'open',configurable:true});
      rejects(()=>c.send('message'));assert(c.bufferedAmount===0);
    });
  }
  for(const [name,callee,owner] of [['child callee',child,window],['main callee',window,child]]) {
    await check(`${name}: genuine foreign data and receiver use callee errors`,()=>{
      const pc=new owner.RTCPeerConnection(),c=pc.createDataChannel('foreign'),send=callee.RTCDataChannel.prototype.send;
      try {
        for(const data of ['message',new owner.Blob(['x']),new owner.ArrayBuffer(2),new owner.Uint8Array(2),new owner.DataView(new owner.ArrayBuffer(2))]) {
          const e=caught(()=>send.call(c,data));assert(e instanceof callee.DOMException && !(e instanceof owner.DOMException) && e.name==='InvalidStateError');
        }
        for(const body of [()=>send.call(c),()=>send.call(c,Symbol()),()=>send.call({},'x'),()=>send.call(c,new owner.ArrayBuffer(2,{maxByteLength:4}))]) {
          const e=caught(body);assert(e instanceof callee.TypeError && !(e instanceof owner.TypeError));
        }
        const sentinel={};assert(caught(()=>send.call(c,{toString(){throw sentinel;}}))===sentinel);
      }finally{pc.close();}
    });
  }
  globalThis.__uiEventResults={complete:true,passed:checks.filter(c=>c.passed).length,total:checks.length,checks};
  return checks.every(c=>c.passed);
})()
