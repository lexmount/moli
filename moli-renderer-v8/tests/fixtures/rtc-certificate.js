(async () => {
  const checks=[];
  const assert=(ok,message='assertion failed')=>{if(!ok)throw Error(message);};
  const equal=(a,b)=>assert(JSON.stringify(a)===JSON.stringify(b),JSON.stringify({actual:a,expected:b}));
  const raises=(action,C,name)=>{let error;try{action();}catch(value){error=value;}assert(error instanceof C && (!name||error.name===name),'expected '+(name||C.name));};
  const realms=[window,document.querySelector('iframe').contentWindow];
  const ec=(extra={})=>({name:'ECDSA',namedCurve:'P-256',...extra});
  const rsa=(w,extra={})=>({name:'RSASSA-PKCS1-v1_5',modulusLength:2048,publicExponent:new w.Uint8Array([1,0,1]),hash:'SHA-256',...extra});
  const fingerprint=cert=>cert.getFingerprints()[0];
  for(const [index,w] of realms.entries()) {
    const C=w.RTCCertificate,P=w.RTCPeerConnection,p=C.prototype,generate=P.generateCertificate;
    const check=async(name,action)=>{
      const key=index+':'+name;globalThis.__codecCurrentCheck=key;
      try {await action();checks.push({name:key,passed:true});}
      catch(error){checks.push({name:key,passed:false,error:{name:error.name,message:error.message}});}
    };
    const rejects=async(action,Constructor,name)=>{
      let promise;try{promise=action();}catch(error){throw Error('synchronous throw: '+error);}
      assert(promise instanceof w.Promise,'callee Promise realm');
      let error;try{await promise;}catch(value){error=value;}
      assert(error instanceof Constructor&&(!name||error.name===name),'expected rejection '+(name||Constructor.name));
    };
    await check('interface and static descriptors',()=>{
      const d=Object.getOwnPropertyDescriptor(w,'RTCCertificate'),s=Object.getOwnPropertyDescriptor(P,'generateCertificate');
      assert(d.writable&&d.configurable&&!d.enumerable&&d.value===C);
      assert(s.writable&&s.configurable&&s.enumerable&&s.value===generate);
      assert(C.name==='RTCCertificate'&&C.length===0&&generate.name==='generateCertificate'&&generate.length===1);
      assert(!Object.hasOwn(p,'generateCertificate')&&Object.getPrototypeOf(p)===w.Object.prototype);
    });
    await check('illegal constructor and nonconstructible static',()=>{
      raises(()=>new C(),w.TypeError);raises(()=>C(),w.TypeError);
      let reads=0;raises(()=>new generate({get name(){reads++;return 'ECDSA';}}),TypeError);assert(reads===0);
    });
    await check('missing required argument rejects',()=>rejects(()=>generate(),w.TypeError));
    for(const [name,value] of [['null',null],['undefined',undefined],['unknown string','does-not-exist'],['unknown object',{name:'unknown'}],['numeric',42]])
      await check('unsupported algorithm '+name,()=>rejects(()=>generate(value),w.DOMException,'NotSupportedError'));
    await check('Symbol input rejects with callee TypeError',()=>rejects(()=>generate(Symbol()),w.TypeError));
    for(const [name,value] of [['missing name',{}],['missing curve',{name:'ECDSA'}],['missing RSA modulus',{name:'RSASSA-PKCS1-v1_5'}],['missing RSA exponent',{name:'RSASSA-PKCS1-v1_5',modulusLength:2048}],['missing RSA hash',{name:'RSASSA-PKCS1-v1_5',modulusLength:2048,publicExponent:new w.Uint8Array([1,0,1])}]])
      await check('required dictionary '+name,()=>rejects(()=>generate(value),w.TypeError));
    for(const curve of ['p-256','P-384','P-521','unknown'])
      await check('unsupported curve '+curve,()=>rejects(()=>generate(ec({namedCurve:curve})),w.DOMException,'NotSupportedError'));
    for(const value of [-1,NaN,Infinity,-Infinity,'invalid',1n,Symbol()])
      await check('expires range '+String(value),()=>rejects(()=>generate(ec({expires:value})),w.TypeError));
    await check('expiration converted before algorithm',async()=>{
      const seen=[],sentinel={};let promise;
      try{promise=generate({get expires(){seen.push('expires');throw sentinel;},get name(){seen.push('name');return 'ECDSA';}});}catch{assert(false,'getter exception must reject');}
      try{await promise;assert(false);}catch(error){assert(error===sentinel);}
      equal(seen,['expires']);
    });
    await check('EC inherited dictionary getter order',async()=>{
      const seen=[],params={};for(const [key,value] of [['expires',10000],['name','eCdSa'],['namedCurve','P-256']])Object.defineProperty(params,key,{get(){seen.push(key);return value;}});
      const cert=await generate.call({},params);assert(cert instanceof C);equal(seen,['expires','name','name','namedCurve']);
    });
    await check('algorithm getter exception is preserved',async()=>{
      const sentinel={};const promise=generate({get name(){throw sentinel;}});assert(promise instanceof w.Promise);
      try{await promise;assert(false);}catch(error){assert(error===sentinel);}
    });
    await check('RSA inherited dictionary getter and hash order',async()=>{
      const seen=[],params={},hash={get name(){seen.push('hash.name');return 'sHa-256';}};
      for(const [key,value] of Object.entries({...rsa(w),hash,expires:10000}))Object.defineProperty(params,key,{get(){seen.push(key);return value;}});
      const cert=await generate(params);assert(cert instanceof C);
      equal(seen,['expires','name','name','modulusLength','publicExponent','hash','hash.name']);
    });
    for(const [name,value] of [['array',[1,0,1]],['ArrayBuffer',new w.ArrayBuffer(3)],['Uint16Array',new w.Uint16Array([1,0,1])],['author Proxy',new Proxy(new w.Uint8Array([1,0,1]),{})],['resizable',new w.Uint8Array(new w.ArrayBuffer(3,{maxByteLength:8}))]])
      await check('RSA exponent native Uint8Array '+name,()=>rejects(()=>generate(rsa(w,{publicExponent:value})),w.TypeError));
    if(typeof w.SharedArrayBuffer==='function')await check('RSA shared exponent rejects',()=>rejects(()=>generate(rsa(w,{publicExponent:new w.Uint8Array(new w.SharedArrayBuffer(3))})),w.TypeError));
    for(const [name,extra] of [['SHA-1',{hash:'SHA-1'}],['unknown hash',{hash:'unknown'}],['large modulus',{modulusLength:0xffffffff}],['zero exponent',{publicExponent:new w.Uint8Array([0])}],['overflow exponent',{publicExponent:new w.Uint8Array([1,0,0,0,1])}]])
      await check('unsupported RSA parameters '+name,()=>rejects(()=>generate(rsa(w,extra)),w.DOMException,'NotSupportedError'));
    await check('RSA exponent subview and leading zeroes',async()=>{
      const buffer=new w.Uint8Array([99,0,0,1,0,1,99]);
      const cert=await generate(rsa(w,{modulusLength:1024,publicExponent:buffer.subarray(1,6)}));assert(cert instanceof C);
    });
    const before=Date.now(),cert=await generate(ec()),after=Date.now();
    await check('native certificate metadata and default lifetime',()=>{
      assert(cert instanceof C&&Object.getPrototypeOf(cert)===p&&Object.prototype.toString.call(cert)==='[object RTCCertificate]');
      assert(Number.isInteger(cert.expires)&&cert.expires>before+2592000000-1000&&cert.expires<=after+2592000000);
      const d=Object.getOwnPropertyDescriptor(p,'expires');assert(typeof d.get==='function'&&d.set===undefined&&d.enumerable&&d.configurable&&!Object.hasOwn(cert,'expires'));
      const m=Object.getOwnPropertyDescriptor(p,'getFingerprints');assert(m.writable&&m.enumerable&&m.configurable&&m.value.length===0&&m.value.name==='getFingerprints');
    });
    await check('fresh fingerprint values and callee realm',()=>{
      const a=cert.getFingerprints(),b=cert.getFingerprints();
      assert(a instanceof w.Array&&a!==b&&a.length>=1&&a[0]!==b[0]&&Object.getPrototypeOf(a[0])===w.Object.prototype);
      equal(a,b);assert(a[0].algorithm==='sha-256'&&/^[0-9a-f]{2}(:[0-9a-f]{2}){31}$/.test(a[0].value));
      equal(Object.keys(a[0]),['algorithm','value']);
      a[0].value='mutated';a[0].algorithm='mutated';a.length=0;assert(cert.getFingerprints()[0].algorithm==='sha-256'&&fingerprint(cert).value===b[0].value);
    });
    await check('certificates have independent random fingerprints',async()=>assert(fingerprint(cert).value!==fingerprint(await generate(ec())).value));
    for(const expires of [0,null,false,0.9])await check('zero lifetime '+String(expires),async()=>{
      const short=await generate(ec({expires}));assert(short.expires<=Date.now());raises(()=>new P({certificates:[short]}),w.DOMException,'InvalidAccessError');
    });
    await check('maximum lifetime is capped',async()=>{
      const start=Date.now(),long=await generate(ec({expires:Number.MAX_SAFE_INTEGER})),end=Date.now();
      assert(long.expires>start+31536000000-1000&&long.expires<=end+31536000000);
    });
    for(const [calleeIndex,callee] of realms.entries()) {
      const getter=Object.getOwnPropertyDescriptor(callee.RTCCertificate.prototype,'expires').get,method=callee.RTCCertificate.prototype.getFingerprints;
      await check('genuine cross realm receiver '+calleeIndex,()=>{
        assert(getter.call(cert)===cert.expires);const result=method.call(cert);assert(result instanceof callee.Array&&Object.getPrototypeOf(result[0])===callee.Object.prototype);
      });
      await check('brand validation without Proxy traps '+calleeIndex,()=>{
        let traps=0;const revoked=Proxy.revocable(cert,{});revoked.revoke();
        for(const receiver of [{},Object.create(p),Object.create(cert),new Proxy(cert,{get(){traps++;throw Error('trap');}}),revoked.proxy]) {
          raises(()=>getter.call(receiver),callee.TypeError);raises(()=>method.call(receiver),callee.TypeError);
        }
        assert(traps===0);
      });
    }
    await check('clone graph identity and private handle preservation',()=>{
      const graph={a:cert,b:cert};graph.self=graph;const copy=w.structuredClone(graph);
      assert(copy!==graph&&copy.self===copy&&copy.a===copy.b&&copy.a!==cert&&copy.a instanceof C&&copy.a.expires===cert.expires);
      equal(copy.a.getFingerprints(),cert.getFingerprints());
      const pc=new P({certificates:[cert]}),clonedPc=new P({certificates:[copy.a]});try{pc.setConfiguration({certificates:[cert]});raises(()=>pc.setConfiguration({certificates:[copy.a]}),w.DOMException,'InvalidModificationError');}finally{pc.close();clonedPc.close();}
    });
    await check('certificate is serializable but not transferable',()=>{
      raises(()=>w.structuredClone(cert,{transfer:[cert]}),w.DOMException,'DataCloneError');assert(cert.expires> Date.now());
      raises(()=>w.structuredClone(new Proxy(cert,{})),w.DOMException,'DataCloneError');
    });
    await check('intrinsic constructors survive replacement',()=>{
      const old=w.RTCCertificate;
      try{w.RTCCertificate=function(){throw Error('public constructor');};const copy=w.structuredClone(cert);assert(Object.getPrototypeOf(copy)===p&&copy.expires===cert.expires);}
      finally{w.RTCCertificate=old;}
    });
    await check('explicit certificates reach SDP via native slots',async()=>{
      const expected=fingerprint(cert).value.toUpperCase(),pc=new P({certificates:[cert]});
      try{
        pc.createDataChannel('certificate');const snapshot=pc.getConfiguration();assert(snapshot.certificates[0]===cert);
        snapshot.certificates.length=0;
        const one=await pc.createOffer();assert(one.sdp.includes('a=fingerprint:sha-256 '+expected+'\r\n'));
        const getter=Object.getOwnPropertyDescriptor(cert,'expires'),fn=Object.getOwnPropertyDescriptor(cert,'getFingerprints');
        try{Object.defineProperty(cert,'expires',{get(){throw Error('public expires');},configurable:true});Object.defineProperty(cert,'getFingerprints',{value(){throw Error('public fingerprints');},configurable:true});
          const two=await pc.createOffer();assert(two.sdp.includes('a=fingerprint:sha-256 '+expected+'\r\n'));
        }finally{if(getter)Object.defineProperty(cert,'expires',getter);else delete cert.expires;if(fn)Object.defineProperty(cert,'getFingerprints',fn);else delete cert.getFingerprints;}
        pc.setConfiguration({certificates:[cert],iceTransportPolicy:'relay'});assert((await pc.createOffer()).sdp.includes(expected));
      }finally{pc.close();}
    });
    await check('default certificates remain stable across configuration updates',async()=>{
      const pc=new P(),other=new P();try{
        pc.createDataChannel('default');other.createDataChannel('other');
        const fingerprints=sdp=>sdp.split('\r\n').filter(line=>line.startsWith('a=fingerprint:'));
        const first=fingerprints((await pc.createOffer()).sdp);assert(first.length>0&&first[0].startsWith('a=fingerprint:sha-256 '));
        pc.setConfiguration({iceTransportPolicy:'relay'});equal(fingerprints((await pc.createOffer()).sdp),first);
        assert(fingerprints((await other.createOffer()).sdp)[0]!==first[0]);equal(pc.getConfiguration().certificates,[]);
      }finally{pc.close();other.close();}
    });
    await check('foreign realm can use a genuine same origin certificate',()=>{const pc=new realms[1-index].RTCPeerConnection({certificates:[cert]});pc.close();});
  }
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  return checks.every(row=>row.passed);
})()
