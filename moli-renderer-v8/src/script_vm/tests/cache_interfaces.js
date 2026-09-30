(async () => {
  const rows=[];
  const assert=(ok,message)=>{if(!ok)throw Error(message);};
  const check=async(name,run)=>{try{await run();rows.push({name,pass:true});}catch(e){rows.push({name,pass:false,error:e.name,message:e.message});}};
  const popup=open();
  const realms=[['main',window],['child',document.getElementById('child').contentWindow],['popup',popup]];
  try {
    for(const [label,realm] of realms) {
      if(!realm.isSecureContext) {
        await check(label+'/insecure',()=>{
          assert(!('Cache' in realm) && !('CacheStorage' in realm),'secure interfaces not exposed');
        });
        continue;
      }
      for(const name of ['Cache','CacheStorage']) await check(label+'/'+name+'/constructor',()=>{
        const C=realm[name],d=Object.getOwnPropertyDescriptor(realm,name);
        assert(typeof C==='function' && C.length===0 && C.name===name,'constructor metadata');
        assert(d.writable && d.configurable && !d.enumerable,'global descriptor');
        assert(Object.getPrototypeOf(C)===realm.Function.prototype && Object.getPrototypeOf(C.prototype)===realm.Object.prototype,'prototype inheritance');
        for(const run of [()=>C(),()=>new C()]){let e;try{run();}catch(error){e=error;}assert(e instanceof realm.TypeError,'illegal constructor');}
      });
      const cacheName='interface-probe-'+label;
      const cache=await realm.caches.open(cacheName);
      try {
        for(const [name,object,methods] of [
          ['CacheStorage',realm.caches,[['match',1],['has',1],['open',1],['delete',1],['keys',0]]],
          ['Cache',cache,[['match',1],['matchAll',0],['put',2],['delete',1],['keys',0]]]
        ]) {
          await check(label+'/'+name+'/instance',()=>{
            const C=realm[name];
            assert(Object.getPrototypeOf(object)===C.prototype && object instanceof C,'native instance prototype');
            assert(Object.prototype.toString.call(object)==='[object '+name+']' && !Object.hasOwn(object,Symbol.toStringTag),'prototype toStringTag');
          });
          for(const [method,length] of methods) await check(label+'/'+name+'/'+method+'/descriptor-and-brand',async()=>{
            const C=realm[name],d=Object.getOwnPropertyDescriptor(C.prototype,method),fn=d.value;
            assert(d.writable && d.enumerable && d.configurable && fn.length===length && fn.name===method,'prototype method descriptor');
            assert(!Object.hasOwn(object,method) && object[method]===fn,'shared method');
            let conversions=0,traps=0;
            const arg={toString(){conversions++;return 'name';}},trap=()=>{traps++;throw Error('trap');};
            const revoked=Proxy.revocable(object,{});revoked.revoke();
            for(const receiver of [{},C.prototype,Object.create(object),new Proxy(object,{get:trap,getPrototypeOf:trap}),revoked.proxy]) {
              const promise=fn.call(receiver,arg,arg);
              assert(promise instanceof realm.Promise,'brand failure rejects a callee-realm promise');
              let error;try{await promise;}catch(e){error=e;}
              assert(error instanceof realm.TypeError,'native receiver rejected');
            }
            assert(conversions===0 && traps===0,'receiver check precedes author conversion and traps');
          });
        }
        await check(label+'/native-roundtrip',async()=>{
          await cache.put('/cache-interface-value',new realm.Response('native cache',{status:201}));
          const response=await cache.match('/cache-interface-value');
          assert(response.status===201 && await response.text()==='native cache','put and match');
          assert((await cache.keys()).length===1,'native keys');
          assert(await cache.delete('/cache-interface-value'),'native delete');
          assert((await cache.keys()).length===0,'native empty cache');
        });
        await check(label+'/author-global-constructor',async()=>{
          const C=realm.Cache,d=Object.getOwnPropertyDescriptor(realm,'Cache');let calls=0;
          Object.defineProperty(realm,'Cache',{configurable:true,get(){calls++;throw Error('author getter');}});
          try {
            const value=await realm.caches.open(cacheName);
            assert(Object.getPrototypeOf(value)===C.prototype && calls===0,'factory uses intrinsic prototype');
          }finally{Object.defineProperty(realm,'Cache',d);}
        });
      }finally{await realm.caches.delete(cacheName);}
    }
  }finally{popup.close();}
  globalThis.__nodeReplacementResults={rows,failures:rows.filter(r=>!r.pass),passed:rows.filter(r=>r.pass).length,total:rows.length};
  return rows.every(r=>r.pass);
})()
