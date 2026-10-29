(async () => {
  async function protocolReference() {
    const checks=[];
    const run=async(name,fn)=>{try{checks.push({name,passed:!!await fn()});}catch(error){checks.push({name,passed:false,detail:String(error.stack||error)});}};
    await run('native for-await does not await the sync result object',async()=>{
      let calls=0,thenGets=0;
      const input={[Symbol.iterator](){return {next(){return calls++===0?{done:false,value:'raw',get then(){thenGets++;throw Error('result then read');}}:{done:true};}};}};
      const values=[];for await(const value of input)values.push(value);
      return thenGets===0&&values.join(',')==='raw';
    });
    await run('native for-await reads the final sync value',async()=>{
      const reason={};let actual;
      const input={[Symbol.iterator](){return {next(){return {done:true,get value(){throw reason;}};}};}};
      try{for await(const value of input){void value;}}catch(error){actual=error;}
      return actual===reason;
    });
    return checks;
  }
  async function exercise(realm,prefix){
    const RS=realm.ReadableStream,P=realm.Promise,checks=[];
    const run=async(name,fn)=>{
      const row={name:`${prefix}/${name}`,passed:false};checks.push(row);
      try{row.passed=!!await fn();}catch(error){row.detail=String(error.stack||error);}
    };
    const input=iterator=>({[Symbol.iterator](){return iterator;}});
    const asyncInput=iterator=>({[Symbol.asyncIterator](){return iterator;}});
    const fence=async()=>{for(let i=0;i<16;i++)await Promise.resolve();};
    const rejection=promise=>promise.then(()=>({fulfilled:true}),reason=>({reason}));
    await run('sync result then getter is ignored',async()=>{
      let calls=0,thenGets=0;
      const stream=RS.from(input({next(){return calls++===0?{done:false,value:'raw',get then(){thenGets++;throw Error('result then read');}}:{done:true};}}));
      const reader=stream.getReader(),first=await reader.read();await reader.cancel();
      return first.value==='raw'&&!first.done&&thenGets===0;
    });
    await run('sync native Promise result uses its own done and value',async()=>{
      const result=P.resolve({done:false,value:'awaited'});result.done=false;result.value='raw';
      const stream=RS.from(input({next(){return result;}})),reader=stream.getReader();
      const first=await reader.read();await reader.cancel();return first.value==='raw'&&!first.done;
    });
    await run('sync final value getter exception propagates',async()=>{
      const reason={},stream=RS.from(input({next(){return {done:true,get value(){throw reason;}};}}));
      return (await rejection(stream.getReader().read())).reason===reason;
    });
    await run('sync final value is awaited before closing',async()=>{
      let resolve,gets=0,settled=false;
      const value=new P(r=>{resolve=r;});
      const reader=RS.from(input({next(){return {done:true,get value(){gets++;return value;}};}})).getReader();
      const pending=reader.read();pending.then(()=>{settled=true;},()=>{settled=true;});
      await fence();const before=!settled&&gets===1;resolve('finished');
      return before&&(await pending).done;
    });
    await run('sync rejected value closes once and keeps original reason',async()=>{
      const reason={},secondary={};let returns=0;
      const value=P.reject(reason);value.catch(()=>{});
      const reader=RS.from(input({next(){return {done:false,value};},return(){returns++;throw secondary;}})).getReader();
      const actual=await rejection(reader.read());return actual.reason===reason&&returns===1;
    });
    await run('sync final rejected value does not close the iterator',async()=>{
      const reason={};let returns=0;const value=P.reject(reason);value.catch(()=>{});
      const reader=RS.from(input({next(){return {done:true,value};},return(){returns++;return {};}})).getReader();
      const actual=await rejection(reader.read());return actual.reason===reason&&returns===0;
    });
    await run('sync value Promise constructor is observed and return errors do not replace it',async()=>{
      const reason={},secondary={};let constructors=0,returns=0;const value=P.resolve('value');
      Object.defineProperty(value,'constructor',{get(){constructors++;throw reason;}});
      const reader=RS.from(input({next(){return {done:false,value};},return(){returns++;throw secondary;}})).getReader();
      const actual=await rejection(reader.read());return actual.reason===reason&&constructors===1&&returns===1;
    });
    await run('sync value Promise same intrinsic constructor skips public then',async()=>{
      let constructors=0,thens=0;const value=P.resolve('value');
      Object.defineProperty(value,'constructor',{get(){constructors++;return P;}});
      Object.defineProperty(value,'then',{get(){thens++;throw Error('public promise then read');}});
      const reader=RS.from(input({next(){return {done:false,value};}})).getReader();
      const first=await reader.read();await reader.cancel();return first.value==='value'&&constructors===1&&thens===0;
    });
    await run('sync done getter exception does not close',async()=>{
      const reason={};let returns=0;
      const reader=RS.from(input({next(){return {get done(){throw reason;}};},return(){returns++;return {};}})).getReader();
      return (await rejection(reader.read())).reason===reason&&returns===0;
    });
    await run('sync value getter exception does not close',async()=>{
      const reason={};let returns=0;
      const reader=RS.from(input({next(){return {done:false,get value(){throw reason;}};},return(){returns++;return {};}})).getReader();
      return (await rejection(reader.read())).reason===reason&&returns===0;
    });
    await run('sync done and value getters run during next',async()=>{
      const trace=[];
      const reader=RS.from(input({next(){trace.push('next');return {get done(){trace.push('done');return false;},get value(){trace.push('value');return 'data';}};}})).getReader();
      await fence();const pending=reader.read(),before=trace.join(',');await pending;await reader.cancel();
      return before==='next,done,value';
    });
    await run('sync return result then getter is ignored',async()=>{
      let gets=0,returns=0;const reason={};let actual;
      const stream=RS.from(input({next(){return {done:false,value:'value'};},return(value){returns++;actual=value;return {done:true,value:'closed',get then(){gets++;throw Error('return then read');}};}}));
      const outcome=await rejection(stream.cancel(reason));return outcome.fulfilled===true&&actual===reason&&returns===1&&gets===0;
    });
    await run('sync return awaits final value and preserves rejection',async()=>{
      const reason={};let returns=0;const value=P.reject(reason);value.catch(()=>{});
      const stream=RS.from(input({next(){return {done:false,value:'value'};},return(){returns++;return {done:true,value};}}));
      return (await rejection(stream.cancel())).reason===reason&&returns===1;
    });
    await run('sync return value getter exception propagates',async()=>{
      const reason={};let returns=0;
      const stream=RS.from(input({next(){return {done:false,value:'value'};},return(){returns++;return {done:true,get value(){throw reason;}};}}));
      return (await rejection(stream.cancel())).reason===reason&&returns===1;
    });
    await run('sync return native Promise result is not awaited',async()=>{
      let calls=0;const value=P.resolve(42);value.done=true;value.value='closed';
      const stream=RS.from(input({next(){return {done:false,value:'value'};},return(){calls++;return value;}}));
      return (await rejection(stream.cancel())).fulfilled===true&&calls===1;
    });
    for(const kind of ['sync','async'])await run(`${kind} non-callable next is rejected on read`,async()=>{
      const sequence=(kind==='sync'?input:asyncInput)({next:0});let stream;
      try{stream=RS.from(sequence);}catch{return false;}
      return (await rejection(stream.getReader().read())).reason instanceof realm.TypeError;
    });
    await run('async result then getter is observed',async()=>{
      let gets=0;
      const reader=RS.from(asyncInput({next(){return {done:false,value:'data',get then(){gets++;return undefined;}};}})).getReader();
      const first=await reader.read();await reader.cancel();return first.value==='data'&&gets===1;
    });
    await run('async yielded Promise is not awaited',async()=>{
      const value=P.resolve('data');
      const reader=RS.from(asyncInput({next(){return P.resolve({done:false,value});}})).getReader();
      const first=await reader.read();await reader.cancel();return first.value===value;
    });
    await run('async final value getter is ignored',async()=>{
      let gets=0;
      const reader=RS.from(asyncInput({next(){return {done:true,get value(){gets++;throw Error('final value read');}};}})).getReader();
      return (await reader.read()).done&&gets===0;
    });
    return checks;
  }
  globalThis.__blobReadingProgress={stage:'protocol',checks:[]};
  const nativeProtocol=await protocolReference(),available=typeof ReadableStream.from==='function';
  if(available){
    const frame=document.createElement('iframe');document.body.appendChild(frame);
    for(const [name,realm] of [['top',window],['child',frame.contentWindow]]){
      __blobReadingProgress.stage=name;
      __blobReadingProgress.checks.push(...await exercise(realm,name));
    }
    frame.remove();
    const code=`(${exercise.toString()})(self,'worker').then(checks=>{postMessage({checks});close();},error=>{postMessage({checks:[{name:'worker completion',passed:false,detail:String(error.stack||error)}]});close();});`;
    const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'})),worker=new Worker(url);
    try{
      const result=await new Promise((resolve,reject)=>{worker.onmessage=event=>resolve(event.data);worker.onerror=event=>reject(Error(event.message));});
      __blobReadingProgress.checks.push(...result.checks);
    }finally{worker.terminate();URL.revokeObjectURL(url);}
  }
  globalThis.__uiEventResults={checks:__blobReadingProgress.checks,nativeProtocol,readableStreamFromAvailable:available};
})().catch(error=>{
  __blobReadingProgress.checks.push({name:'fixture completion',passed:false,detail:String(error.stack||error)});
  globalThis.__uiEventResults={checks:__blobReadingProgress.checks};
});
