(async () => {
  const checks = [];
  async function check(name, test) {
    try { checks.push({name, passed: (await test()) === true}); }
    catch (error) { checks.push({name, passed:false, error:String(error)}); }
  }
  const names = ['cache-order-z','cache-order-a','cache-order-Z','cache-order-',
    'cache-order-\ud800','cache-order-\ufffd','cache-order-\0','cache-order-😀'];
  const list = async () => (await caches.keys()).filter(name => names.includes(name));
  const same = (a,b) => JSON.stringify(a) === JSON.stringify(b);
  const handles = [];
  try {
    await Promise.all(names.map(name => caches.delete(name)));
    handles.push(...await Promise.all(names.map(name => caches.open(name))));
    await check('names follow creation order', async () => same(await list(),names));
    await caches.open(names[2]);
    await check('reopen preserves position', async () => same(await list(),names));
    for (let i=0;i<names.length;i++) await handles[i].put('/ordering',new Response(names[i]));
    await check('storage match selects oldest cache', async () => (await (await caches.match('/ordering')).text()) === names[0]);
    await check('storage explicit name selects chosen cache', async () => (await (await caches.match('/ordering',{cacheName:names[1]})).text()) === names[1]);
    await check('delete existing cache', async () => await caches.delete(names[1]));
    await check('delete preserves survivor order', async () => same(await list(),names.filter((_,i)=>i!==1)));
    const recreated = await caches.open(names[1]);
    await check('recreated name goes last', async () => same(await list(),[...names.filter((_,i)=>i!==1),names[1]]));
    await check('recreated cache is empty', async () => (await recreated.keys()).length===0);
    await check('deleted handle keeps contents', async () => await (await handles[1].match('/ordering')).text()===names[1]);
    await caches.delete(names[0]);
    await check('storage match follows next surviving cache', async () => await (await caches.match('/ordering')).text()===names[2]);
    await check('missing named match does not create cache', async () => {
      const before=await list();const missing='cache-order-absent';await caches.delete(missing);
      return await caches.match('/ordering',{cacheName:missing})===undefined && !(await caches.has(missing)) && same(await list(),before);
    });
    await check('storage keys returns fresh mutable array', async () => {
      const a=await caches.keys(),b=await caches.keys();a.push('local-only');return a!==b && !Object.isFrozen(a) && !(await caches.has('local-only'));
    });
    const cache=await caches.open('cache-results');
    await cache.put('/first',new Response('first',{headers:{'x-value':'1'}}));
    await cache.put('/second',new Response('second',{headers:{'x-value':'2'}}));
    for (const method of ['keys','matchAll']) {
      for (const request of [undefined,'/first','/missing']) {
        const label=method+'/'+String(request),a=await cache[method](request),b=await cache[method](request);
        await check(label+'/frozen array', () => Array.isArray(a)&&Object.isFrozen(a));
        await check(label+'/immutable length and entries', async () => {
          const value=await cache[method](request),old=value.length;
          return !Reflect.set(value,'length',old+1)&&!Reflect.set(value,'0',null)&&value.length===old;
        });
        await check(label+'/fresh result and elements', () => a!==b&&(a.length===0||a[0]!==b[0]));
        await check(label+'/shallow freeze', () => {if(!a.length)return true;a[0].marker=42;return a[0].marker===42&&!Object.isFrozen(a[0]);});
        if (a.length) for (const mutation of ['append','set','delete']) {
          await check(label+'/immutable headers '+mutation, () => {
            let error;try{a[0].headers[mutation]('x-value','changed');}catch(e){error=e;}return error instanceof TypeError;
          });
        }
      }
      await check(method+'/internal headers ignore author iterator', async () => {
        const descriptor=Object.getOwnPropertyDescriptor(Array.prototype,Symbol.iterator);let reads=0;
        Object.defineProperty(Array.prototype,Symbol.iterator,{configurable:true,get(){reads++;throw 42;}});
        try{return (await cache[method]()).length===2&&reads===0;}finally{Object.defineProperty(Array.prototype,Symbol.iterator,descriptor);}
      });
      const ctor=method==='keys'?'Request':'Response',descriptor=Object.getOwnPropertyDescriptor(globalThis,ctor);
      await check(method+'/intrinsic constructor', async () => {
        let calls=0;Object.defineProperty(globalThis,ctor,{configurable:true,get(){calls++;throw 42;}});
        try{return (await cache[method]()).length===2&&calls===0;}finally{Object.defineProperty(globalThis,ctor,descriptor);}
      });
    }
    await check('keys ignores inherited RequestInit getters',async()=>{
      const descriptor=Object.getOwnPropertyDescriptor(Object.prototype,'mode');let reads=0;
      Object.defineProperty(Object.prototype,'mode',{configurable:true,get(){reads++;throw 42;}});
      try{return (await cache.keys()).length===2&&reads===0;}finally{if(descriptor)Object.defineProperty(Object.prototype,'mode',descriptor);else delete Object.prototype.mode;}
    });
    if(typeof document!=='undefined') {
      const frame=document.createElement('iframe');document.body.append(frame);const child=frame.contentWindow;
      try {
        const childCache=await child.caches.open('cache-results');
        for(const [ownerLabel,owner,receiver] of [['main',globalThis,cache],['child',child,childCache]]) {
          for(const [calleeLabel,callee] of [['main',globalThis],['child',child]]) {
            for(const method of ['keys','matchAll']) {
              const label='realm/'+ownerLabel+'/'+calleeLabel+'/'+method;
              const promise=callee.Cache.prototype[method].call(receiver),a=await promise;
              const C=method==='keys'?owner.Request:owner.Response;
              await check(label+'/promise',()=>Object.getPrototypeOf(promise)===owner.Promise.prototype);
              await check(label+'/array',()=>Object.getPrototypeOf(a)===owner.Array.prototype);
              await check(label+'/elements',()=>a.length===2&&a.every(value=>Object.getPrototypeOf(value)===C.prototype));
              await check(label+'/frozen',()=>Object.isFrozen(a));
              await check(label+'/invalid URL callee rejection',async()=>{
                const p=callee.Cache.prototype[method].call(receiver,'http://[');let error;
                try{await p;}catch(e){error=e;}
                return Object.getPrototypeOf(p)===callee.Promise.prototype&&error instanceof callee.TypeError;
              });
              await check(label+'/getter callee rejection',async()=>{
                const reason={};const p=callee.Cache.prototype[method].call(receiver,'/first',{get ignoreMethod(){throw reason;}});let error;
                try{await p;}catch(e){error=e;}
                return Object.getPrototypeOf(p)===callee.Promise.prototype&&error===reason;
              });
            }
            await check('realm/'+ownerLabel+'/'+calleeLabel+'/match',async()=>{
              const value=await callee.Cache.prototype.match.call(receiver,'/first');return Object.getPrototypeOf(value)===owner.Response.prototype;
            });
            for(const method of ['keys','match','open','has','delete']) {
              const input=method==='match'?'/first':method==='delete'?'cache-order-absent':'cache-results';
              const promise=callee.CacheStorage.prototype[method].call(owner.caches,input),value=await promise;
              const label='storage-realm/'+ownerLabel+'/'+calleeLabel+'/'+method;
              await check(label+'/promise',()=>Object.getPrototypeOf(promise)===owner.Promise.prototype);
              await check(label+'/value',()=>{
                if(method==='keys')return Object.getPrototypeOf(value)===owner.Array.prototype&&!Object.isFrozen(value);
                if(method==='match')return Object.getPrototypeOf(value)===owner.Response.prototype;
                if(method==='open')return Object.getPrototypeOf(value)===owner.Cache.prototype;
                return typeof value==='boolean';
              });
            }
          }
        }
      }finally{frame.remove();}
    }
  } finally {
    await Promise.all([...names,'cache-results','cache-order-absent'].map(name=>caches.delete(name)));
  }
  globalThis.__uiEventResults={checks,total:checks.length,passed:checks.filter(row=>row.passed).length,complete:true};
  return true;
})()
