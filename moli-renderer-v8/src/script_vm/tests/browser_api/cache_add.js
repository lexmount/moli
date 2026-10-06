(async () => {
  const facts = globalThis.__uiEventResults = {complete:false,checks:[]};
  async function check(name, run) {
    try { if (await run() !== true) throw Error('assertion'); facts.checks.push({name,passed:true}); }
    catch(error) { facts.checks.push({name,passed:false,error:String(error),stack:error?.stack}); }
  }
  const name = 'cache-batch-fixture';
  const cache = await caches.open(name);
  const rejects = async (run, kind) => { let value; try { await run(); } catch(error) {value=error;} return value instanceof kind; };
  const resource = (name, extra='') => '/batch-resource?key='+encodeURIComponent(name)+extra;
  const req = (url,shape) => new Request(url,{headers:{'x-shape':shape}});
  try {
    for (const member of ['add','addAll']) {
      await check(member+' descriptor', () => {
        const descriptor=Object.getOwnPropertyDescriptor(Cache.prototype,member);
        return typeof descriptor.value==='function'&&descriptor.value.length===1&&descriptor.value.name===member&&descriptor.enumerable&&descriptor.writable&&descriptor.configurable;
      });
      const revoked=Proxy.revocable(cache,{});revoked.revoke();
      for (const [label,receiver] of [['ordinary',{}],['prototype',Cache.prototype],['forged',Object.create(Cache.prototype)],['inherited',Object.create(cache)],['author proxy',new Proxy(cache,{})],['revoked',revoked.proxy]]) {
        await check(member+' receiver '+label,async()=>{
          let conversions=0;
          const item={toString(){conversions++;throw 42;}};
          let promise;
          try { promise=Cache.prototype[member].call(receiver,member==='add'?item:[item]); } catch { return false; }
          return promise instanceof Promise&&await rejects(()=>promise,TypeError)&&conversions===0;
        });
      }
      await check(member+' no argument',()=>rejects(()=>cache[member](),TypeError));
    }
    for (const input of [undefined,null,42,true,'abc',Symbol('x'),{}]) {
      await check('addAll non sequence '+String(input),()=>rejects(()=>cache.addAll(input),TypeError));
    }
    await check('empty sequence',async()=>await cache.addAll([])===undefined&&(await cache.keys()).length===0);
    await check('iterable converts once',async()=>{
      let count=0;
      function* items(){yield {toString(){count++;return resource('iterable');}};}
      await cache.addAll(items());return count===1&&(await cache.match(resource('iterable'))).status===200;
    });
    await check('iterable conversion preserves reason without closing',async()=>{
      const reason={sentinel:42};let closed=0,converted=0;
      function* items(){try{yield {toString(){converted++;return resource('first');}};yield {toString(){throw reason;}};}finally{closed++;}}
      let error;try{await cache.addAll(items());}catch(caught){error=caught;}
      return error===reason&&closed===0&&converted===1&&await cache.match(resource('first'))===undefined;
    });
    await check('sequence conversion before Request validation',async()=>{
      const reason={order:true};let error;
      try{await cache.addAll([new Request(resource('post'),{method:'POST'}),{toString(){throw reason;}}]);}catch(caught){error=caught;}
      return error===reason;
    });
    for (const member of ['add','addAll']) {
      const call=value=>member==='add'?cache.add(value):cache.addAll([value]);
      for (const url of ['data:text/plain,text','javascript:0','file:///missing']) {
        await check(member+' rejects scheme '+url,()=>rejects(()=>call(url),TypeError));
      }
      for (const method of ['POST','HEAD','PUT']) {
        await check(member+' rejects '+method,()=>rejects(()=>call(new Request(resource(method),{method})),TypeError));
      }
      for (const status of [206,404,500]) {
        await check(member+' rejects status '+status,()=>rejects(()=>call(resource('status-'+member+'-'+status,'&status='+status)),TypeError));
      }
      await check(member+' rejects Vary star',()=>rejects(()=>call(resource('star-'+member,'&vary=*')),TypeError));
      await check(member+' null response body',async()=>{
        const url=resource('empty-'+member,'&status=204');
        return await call(url)===undefined&&(await cache.match(url)).status===204&&(await (await cache.match(url)).text())==='';
      });
      await check(member+' abort reason identity',async()=>{
        const reason={cancel:member};const controller=new AbortController();controller.abort(reason);
        const request=new Request(resource('abort-'+member),{signal:controller.signal});let error;
        try{await call(request);}catch(caught){error=caught;}return error===reason&&await cache.match(request)===undefined;
      });
      await check(member+' synchronous abort reason identity',async()=>{
        const reason={late:member};const controller=new AbortController();
        const request=new Request(resource('late-'+member),{signal:controller.signal});
        const promise=call(request);controller.abort(reason);let error;
        try{await promise;}catch(caught){error=caught;}return error===reason&&await cache.match(request)===undefined;
      });
    }
    await check('batch failure keeps previous contents',async()=>{
      const old=resource('preserved');await cache.put(old,new Response('old'));
      const before=(await cache.keys()).map(r=>r.url);
      let failed=await rejects(()=>cache.addAll([resource('good'),resource('bad','&status=500')]),TypeError);
      return failed&&JSON.stringify((await cache.keys()).map(r=>r.url))===JSON.stringify(before)&&await(await cache.match(old)).text()==='old'&&await cache.match(resource('good'))===undefined;
    });
    await check('duplicate batch rolls back replacements',async()=>{
      const url=resource('duplicate');await cache.put(url,new Response('previous'));
      let error;try{await cache.addAll([url,url+'#same']);}catch(caught){error=caught;}
      return error instanceof DOMException&&error.name==='InvalidStateError'&&await(await cache.match(url)).text()==='previous';
    });
    const vary=resource('vary','&vary=x-shape');
    const circle=req(vary,'circle'),square=req(vary,'square');
    await check('Vary batch allows distinct variants',async()=>await cache.addAll([circle,square])===undefined);
    await check('Vary entries preserve request headers',async()=>{
      const entries=await cache.keys(vary,{ignoreVary:true});return entries.length===2&&entries[0].headers.get('x-shape')==='circle'&&entries[1].headers.get('x-shape')==='square';
    });
    await check('Vary match selects circle',async()=>await(await cache.match(circle)).text()==='circle');
    await check('Vary match selects square',async()=>await(await cache.match(square)).text()==='square');
    await check('Vary matchAll returns both',async()=>JSON.stringify(await Promise.all((await cache.matchAll(vary,{ignoreVary:true})).map(r=>r.text())))==='["circle","square"]');
    await check('Vary replacement retains sibling',async()=>{
      await cache.put(circle,new Response('updated',{headers:{Vary:'x-shape'}}));
      return await(await cache.match(circle)).text()==='updated'&&await(await cache.match(square)).text()==='square'&&JSON.stringify((await cache.keys(vary,{ignoreVary:true})).map(r=>r.headers.get('x-shape')))==='["square","circle"]';
    });
    await check('Vary delete retains sibling',async()=>await cache.delete(circle)&&await cache.match(circle)===undefined&&await(await cache.match(square)).text()==='square');
    await check('Vary duplicate rolls back',async()=>{
      let error;try{await cache.addAll([square,square]);}catch(caught){error=caught;}
      return error?.name==='InvalidStateError'&&await(await cache.match(square)).text()==='square';
    });
    await check('native Fetch and Request ignore mutable globals',async()=>{
      const OriginalRequest=Request,originalFetch=fetch;let calls=0;
      try {
        globalThis.Request=()=>{calls++;throw 43;};globalThis.fetch=()=>{calls++;throw 44;};
        const url=resource('intrinsic');await cache.addAll([url,new OriginalRequest(resource('intrinsic-request'))]);
        return calls===0&&(await cache.match(url)).status===200;
      } finally {globalThis.Request=OriginalRequest;globalThis.fetch=originalFetch;}
    });
    await check('Request native slots ignore author getters',async()=>{
      const request=new Request(resource('native-slots'));let reads=0;
      for(const key of ['url','method','headers','signal','mode','credentials','redirect'])Object.defineProperty(request,key,{get(){reads++;throw 45;}});
      await cache.add(request);return reads===0&&(await cache.match(resource('native-slots'))).status===200;
    });
    await check('Request headers snapshot precedes asynchronous mutation',async()=>{
      const request=req(resource('headers','&vary=x-shape'),'old');const promise=cache.add(request);request.headers.set('x-shape','new');await promise;
      return await(await cache.match(req(resource('headers','&vary=x-shape'),'old'))).text()==='old'&&await cache.match(request)===undefined;
    });
  } finally {await caches.delete(name);}
  facts.complete=true;facts.total=facts.checks.length;facts.passed=facts.checks.filter(row=>row.passed).length;
  return true;
})()
