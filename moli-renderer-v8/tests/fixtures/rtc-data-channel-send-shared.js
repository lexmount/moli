(async () => {
  const checks=[];
  const assert=(ok,message='assertion failed')=>{if(!ok)throw Error(message);};
  const caught=body=>{try{body();}catch(error){return error;}};
  const check=async(name,body)=>{try{await body();checks.push({name,passed:true});}catch(error){checks.push({name,passed:false,error:String(error),stack:error.stack});}};
  const child=document.querySelector('iframe').contentWindow;
  for(const [realm,w] of [['main',window],['iframe',child]]) {
    const run=(name,body)=>check(`${realm}: ${name}`,()=>{
      assert(typeof w.SharedArrayBuffer==='function','SharedArrayBuffer must be exposed in this isolated fixture');
      const pc=new w.RTCPeerConnection(),c=pc.createDataChannel('shared');
      try{body(c);}finally{pc.close();}
    });
    const rejects=body=>{const e=caught(body);assert(e instanceof w.TypeError,`${e}`);};
    await run('fixed shared buffers and all shared views reject before state',c=>{
      const b=new w.SharedArrayBuffer(16);
      const values=[b,new w.DataView(b)];
      for(const name of ['Int8Array','Uint8Array','Uint8ClampedArray','Int16Array','Uint16Array','Int32Array','Uint32Array','Float32Array','Float64Array','BigInt64Array','BigUint64Array'])values.push(new w[name](b));
      for(const value of values)rejects(()=>c.send(value));
      assert(c.bufferedAmount===0);
    });
    await run('growable shared buffers and fixed and tracking views reject before state',c=>{
      const b=new w.SharedArrayBuffer(8,{maxByteLength:16});
      for(const value of [b,new w.Uint8Array(b),new w.Uint8Array(b,0,4),new w.DataView(b),new w.DataView(b,0,4)])rejects(()=>c.send(value));
    });
    await run('shared classification never reads author buffer properties or stringifier',c=>{
      const b=new w.SharedArrayBuffer(8);let reads=0;
      for(const value of [b,new w.Uint8Array(b),new w.DataView(b)]) {
        for(const key of ['buffer','byteLength','growable','resizable',Symbol.toPrimitive])w.Object.defineProperty(value,key,{get(){reads++;throw Error('shared getter');}});
        rejects(()=>c.send(value));
      }
      assert(reads===0);
    });
    await run('author Proxy around shared buffers selects observable string overload',c=>{
      const b=new w.SharedArrayBuffer(8);let reads=0,calls=0;
      const proxy=new w.Proxy(b,{get(target,key){reads++;assert(key===Symbol.toPrimitive);return hint=>{calls++;assert(hint==='string');return 'shared proxy';};}});
      const error=caught(()=>c.send(proxy));
      assert(error instanceof w.DOMException && error.name==='InvalidStateError' && reads===1 && calls===1);
    });
    await run('closed channel still rejects shared arguments during conversion',c=>{
      c.close();for(const value of [new w.SharedArrayBuffer(0),new w.Uint8Array(new w.SharedArrayBuffer(8))])rejects(()=>c.send(value));
      assert(c.bufferedAmount===0);
    });
  }
  for(const [name,callee,owner] of [['child callee',child,window],['main callee',window,child]]) {
    await check(`${name}: foreign shared arguments use callee TypeError`,()=>{
      assert(typeof owner.SharedArrayBuffer==='function');
      const pc=new owner.RTCPeerConnection(),c=pc.createDataChannel('shared realm');
      try{for(const value of [new owner.SharedArrayBuffer(8),new owner.Uint8Array(new owner.SharedArrayBuffer(8)),new owner.DataView(new owner.SharedArrayBuffer(8))]) {
        const error=caught(()=>callee.RTCDataChannel.prototype.send.call(c,value));
        assert(error instanceof callee.TypeError && !(error instanceof owner.TypeError));
      }}finally{pc.close();}
    });
  }
  globalThis.__uiEventResults={complete:true,passed:checks.filter(c=>c.passed).length,total:checks.length,checks};
  return checks.every(c=>c.passed);
})()
