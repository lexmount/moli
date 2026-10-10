(async()=>{
 const checks=[],child=document.querySelector('iframe').contentWindow;
 const assert=(ok,message='assertion failed')=>{if(!ok)throw Error(message);};
 const caught=body=>{try{body();}catch(error){return error;}};
 const tick=()=>new Promise(resolve=>setTimeout(resolve,40));
 const check=async(name,body)=>{try{await body();checks.push({name,passed:true});}catch(error){checks.push({name,passed:false,error:String(error),stack:error.stack});}};
 const state=(w,c)=>w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype,'readyState').get.call(c);
 // A listener's resolved Promise can resume before dispatch has finished.
 // Inspect final event state and later listeners from a subsequent task.
 const watch=(w,c)=>{
  const events=[];let resolve;const closed=new Promise(r=>{resolve=r;});
  for(const type of ['open','closing','close','error'])c.addEventListener(type,e=>{
   events.push({event:e,type:e.type,state:state(w,c),target:e.target,current:e.currentTarget,phase:e.eventPhase});
   if(type==='close')resolve(e);
  });
  return {events,wait:()=>{let timer;return Promise.race([closed,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('close event timeout')),1000);})]).finally(()=>clearTimeout(timer)).then(async event=>{await tick();return event;});}};
 };
 for(const [realm,w] of [['main',window],['iframe',child]]) {
  const run=(name,body)=>check(`${realm}: ${name}`,async()=>{
   const pc=new w.RTCPeerConnection();try{await body(pc);}finally{w.RTCPeerConnection.prototype.close.call(pc);}
  });
  await run('close descriptor and ignored arguments',pc=>{
   const d=w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype,'close'),c=pc.createDataChannel('descriptor');let conversions=0;
   assert(d.value.name==='close'&&d.value.length===0&&d.writable&&d.enumerable&&d.configurable&&!w.Object.hasOwn(c,'close'));
   assert(c.close({toString(){conversions++;throw Error('extra');}})===undefined&&conversions===0);
   assert(state(w,c)==='closing');
  });
  const handlers=['onopen','onclosing','onclose','onmessage','onerror','onbufferedamountlow'];
  await run('all six handlers have native prototype accessors and null defaults',pc=>{
   const c=pc.createDataChannel('handler descriptors');
   for(const name of handlers){const d=w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype,name);
    assert(d&&d.enumerable&&d.configurable&&typeof d.get==='function'&&typeof d.set==='function');
    assert(d.get.name==='get '+name&&d.get.length===0&&d.set.name==='set '+name&&d.set.length===1&&!w.Object.hasOwn(c,name)&&c[name]===null);
   }
  });
  await run('handler values retain objects and clear primitives without conversion',pc=>{
   const c=pc.createDataChannel('handler values');let calls=0,reads=0;
   const object={get handleEvent(){reads++;throw Error('handleEvent');},toString(){reads++;throw Error('conversion');}};
   for(const name of handlers){const fn=function(e){assert(this===c&&e.type===name.slice(2));calls++;};
    c[name]=fn;assert(c[name]===fn);c.dispatchEvent(new w.Event(name.slice(2)));
    c[name]=object;assert(c[name]===object);c.dispatchEvent(new w.Event(name.slice(2)));
    for(const value of [undefined,null,false,1,1n,'text',Symbol()]){c[name]=value;assert(c[name]===null);}
   }
   assert(calls===6&&reads===0&&state(w,c)==='connecting');
  });
  await run('handler accessors reject forged and author Proxy receivers before traps',pc=>{
   const c=pc.createDataChannel('handler receivers');let reads=0;
   const proxy=new w.Proxy(c,{get(){reads++;throw Error('proxy');},getPrototypeOf(){reads++;throw Error('proxy');}}),revoked=w.Proxy.revocable(c,{});revoked.revoke();
   const value={toString(){reads++;throw Error('conversion');}};
   for(const name of handlers){const d=w.Object.getOwnPropertyDescriptor(w.RTCDataChannel.prototype,name);
    for(const receiver of [{},w.Object.create(w.RTCDataChannel.prototype),w.Object.create(c),proxy,revoked.proxy,null,undefined]){
     assert(caught(()=>d.get.call(receiver)) instanceof w.TypeError);assert(caught(()=>d.set.call(receiver,value)) instanceof w.TypeError);
    }
    assert(c[name]===null);
   }
   assert(reads===0&&state(w,c)==='connecting');
  });
  await run('a noncallable close handler is retained without reading handleEvent',async pc=>{
   const c=pc.createDataChannel('noncallable'),completion=watch(w,c);let reads=0;
   const value={get handleEvent(){reads++;throw Error('handleEvent');}};c.onclose=value;c.close();await completion.wait();
   assert(c.onclose===value&&reads===0&&completion.events.length===1&&state(w,c)==='closed');
  });
  for(const options of [{},{negotiated:true,id:7}]) {
   const mode=options.negotiated?'negotiated':'in-band';
   await run(`${mode} local close enters closing synchronously`,async pc=>{
    const c=pc.createDataChannel('sync',options),trace=watch(w,c),sibling=pc.createDataChannel('sibling');
    assert(c.close()===undefined&&state(w,c)==='closing'&&trace.events.length===0);
    await Promise.resolve();assert(state(w,c)==='closing'&&trace.events.length===0);
    assert(pc.signalingState==='stable'&&state(w,sibling)==='connecting');await trace.wait();
   });
   await run(`${mode} completion dispatches one trusted event in the target realm`,async pc=>{
    const c=pc.createDataChannel('event',options),trace=watch(w,c);c.close();const e=await trace.wait();
    assert(state(w,c)==='closed'&&trace.events.length===1);
    const row=trace.events[0];assert(row.type==='close'&&row.state==='closed'&&row.target===c&&row.current===c&&row.phase===w.Event.AT_TARGET);
    assert(e instanceof w.Event&&w.Object.getPrototypeOf(e)===w.Event.prototype&&e.isTrusted&&!e.bubbles&&!e.cancelable&&!e.composed);
    assert(e.currentTarget===null&&e.eventPhase===w.Event.NONE&&c.bufferedAmount===0);
   });
   await run(`${mode} repeated close before and after completion is idempotent`,async pc=>{
    const c=pc.createDataChannel('repeat',options),trace=watch(w,c);c.close();c.close();await Promise.resolve();c.close();
    await trace.wait();c.close();c.close();await tick();assert(trace.events.length===1&&state(w,c)==='closed');
   });
   await run(`${mode} close is reentrant from its close handler`,async pc=>{
    const c=pc.createDataChannel('reentrant',options),trace=watch(w,c);let calls=0;
    c.onclose=function(){calls++;assert(this===c&&state(w,c)==='closed');c.close();};c.close();await trace.wait();await tick();assert(calls===1&&trace.events.length===1);
   });
   await run(`${mode} closing one channel preserves its connection and siblings`,async pc=>{
    const c=pc.createDataChannel('one',options),sibling=pc.createDataChannel('two'),trace=watch(w,c),other=watch(w,sibling);
    c.binaryType='blob';c.bufferedAmountLowThreshold=37;c.close();await trace.wait();
    assert(pc.signalingState==='stable'&&state(w,sibling)==='connecting'&&other.events.length===0);
    assert(c.binaryType==='blob'&&c.bufferedAmountLowThreshold===37&&c.bufferedAmount===0);
   });
  }
  await run('close handlers preserve listener order and task microtask boundaries',async pc=>{
   const c=pc.createDataChannel('order'),trace=[],completion=watch(w,c);
   c.addEventListener('close',()=>trace.push('first'));
   c.onclose=()=>{trace.push('handler');Promise.resolve().then(()=>trace.push('reaction'));};
   c.addEventListener('close',()=>trace.push('last'));c.close();assert(trace.length===0);await completion.wait();
   assert(trace.filter(value=>value!=='reaction').join('|')==='first|handler|last');
   assert(trace.filter(value=>value==='reaction').length===1&&trace.indexOf('reaction')>trace.indexOf('handler'));
  });
  await run('replacing the handler keeps its registration position',async pc=>{
   const c=pc.createDataChannel('replace'),trace=[],completion=watch(w,c);
   c.onclose=()=>trace.push('old');c.addEventListener('close',()=>trace.push('listener'));c.onclose=()=>trace.push('new');
   c.close();await completion.wait();assert(trace.join('|')==='new|listener');
  });
  await run('clearing the handler before the task suppresses only that handler',async pc=>{
   const c=pc.createDataChannel('clear'),completion=watch(w,c);let count=0;c.onclose=()=>count++;
   c.close();c.onclose=null;await completion.wait();assert(count===0&&completion.events.length===1);
  });
  await run('a listener registered after close still observes the queued event',async pc=>{
   const c=pc.createDataChannel('late');c.close();const completion=watch(w,c);await completion.wait();assert(completion.events.length===1);
  });
  await run('private closing state ignores author attribute getters',async pc=>{
   const c=pc.createDataChannel('spoof'),completion=watch(w,c);let reads=0;
   for(const name of ['readyState','id','label','ordered','negotiated'])w.Object.defineProperty(c,name,{get(){reads++;throw Error('author getter');},configurable:true});
   c.close();assert(state(w,c)==='closing');await completion.wait();assert(state(w,c)==='closed'&&reads===0);
  });
  await run('native channel registries bypass author Set constructors and methods',async pc=>{
   const original=w.Set,methods=w.Object.fromEntries(['add','delete','clear','has','values'].map(name=>[name,original.prototype[name]]));
   const c=pc.createDataChannel('intrinsic set'),completion=watch(w,c);let calls=0;
   try{
    w.Set=function(){calls++;throw Error('Set constructor');};for(const name of w.Object.keys(methods))original.prototype[name]=function(){calls++;throw Error('Set method');};
    c.close();await completion.wait();assert(calls===0&&state(w,c)==='closed');
   }finally{w.Set=original;for(const [name,value] of w.Object.entries(methods))original.prototype[name]=value;}
  });
  await run('close event construction uses the intrinsic target Event',async pc=>{
   const original=w.Event,c=pc.createDataChannel('intrinsic event'),completion=watch(w,c);let calls=0;
   try{w.Event=function(){calls++;throw Error('Event constructor');};c.close();const e=await completion.wait();assert(e instanceof original&&w.Object.getPrototypeOf(e)===original.prototype&&calls===0);}
   finally{w.Event=original;}
  });
  await run('send validates conversions in both closing and closed states',async pc=>{
   const c=pc.createDataChannel('send'),completion=watch(w,c);c.close();let count=0;
   for(const name of ['closing','closed']){
    assert(state(w,c)===name);const e=caught(()=>c.send({toString(){count++;return 'payload';}}));
    assert(e instanceof w.DOMException&&e.name==='InvalidStateError'&&e.code===11);
    assert(caught(()=>c.send(Symbol())) instanceof w.TypeError&&c.bufferedAmount===0);if(name==='closing')await completion.wait();
   }
   assert(count===2);
  });
  await run('the close listener can reuse the released negotiated id',async pc=>{
   const c=pc.createDataChannel('old',{negotiated:true,id:59}),completion=watch(w,c);let next,error;
   c.addEventListener('close',()=>{try{next=pc.createDataChannel('next',{negotiated:true,id:59});}catch(e){error=e;}});
   c.close();await completion.wait();assert(!error&&next&&next.id===59&&state(w,next)==='connecting'&&state(w,c)==='closed');
  });
  await run('PC close synchronously closes all local channels without channel events',async pc=>{
   const a=pc.createDataChannel('a'),b=pc.createDataChannel('b',{negotiated:true,id:71}),ta=watch(w,a),tb=watch(w,b);
   pc.close();assert(state(w,a)==='closed'&&state(w,b)==='closed'&&pc.signalingState==='closed'&&pc.connectionState==='closed'&&pc.iceConnectionState==='closed');
   await Promise.resolve();await tick();assert(ta.events.length===0&&tb.events.length===0);
  });
  await run('PC shutdown never calls channel methods or author properties',async pc=>{
   const c=pc.createDataChannel('private PC close');let calls=0;
   c.close=()=>{calls++;throw Error('author close');};for(const name of ['readyState','id','label'])w.Object.defineProperty(c,name,{get(){calls++;throw Error('author property');}});
   w.RTCPeerConnection.prototype.close.call(pc);assert(state(w,c)==='closed'&&calls===0);
  });
  await run('PC close suppresses an already requested local close event',async pc=>{
   const c=pc.createDataChannel('pending'),completion=watch(w,c);c.close();pc.close();
   assert(state(w,c)==='closed');await tick();assert(completion.events.length===0);
  });
  await run('repeated PC close after completed channel close cannot emit more events',async pc=>{
   const c=pc.createDataChannel('completed'),completion=watch(w,c);c.close();await completion.wait();pc.close();pc.close();c.close();await tick();
   assert(state(w,c)==='closed'&&completion.events.length===1);
  });
  await run('PC close ignores extra arguments and validates native receivers',async pc=>{
   const c=pc.createDataChannel('args');let conversions=0,reads=0;const extra={toString(){conversions++;throw Error('extra');}};
   const proxy=new w.Proxy(pc,{get(){reads++;throw Error('proxy');},getPrototypeOf(){reads++;throw Error('proxy');}});
   const revoked=w.Proxy.revocable(pc,{});revoked.revoke();
   for(const receiver of [{},w.Object.create(w.RTCPeerConnection.prototype),w.Object.create(pc),proxy,revoked.proxy,null,undefined])assert(caught(()=>w.RTCPeerConnection.prototype.close.call(receiver,extra)) instanceof w.TypeError);
   assert(state(w,c)==='connecting'&&pc.signalingState==='stable'&&reads===0&&conversions===0);
   assert(pc.close(extra)===undefined&&conversions===0&&pc.signalingState==='closed');await tick();assert(state(w,c)==='closed');
  });
  await run('closing one PC leaves a different PC and its channels intact',pc=>{
   const other=new w.RTCPeerConnection();try{
    const a=pc.createDataChannel('a'),b=other.createDataChannel('b');pc.close();assert(state(w,a)==='closed'&&state(w,b)==='connecting'&&other.signalingState==='stable');
   }finally{other.close();}
  });
  await run('channel close rejects forged and author Proxy receivers before traps',pc=>{
   const c=pc.createDataChannel('receiver');let reads=0,conversions=0;
   const proxy=new w.Proxy(c,{get(){reads++;throw Error('proxy');},getPrototypeOf(){reads++;throw Error('proxy');}}),revoked=w.Proxy.revocable(c,{});revoked.revoke();
   const extra={toString(){conversions++;throw Error('extra');}};
   for(const receiver of [{},w.Object.create(w.RTCDataChannel.prototype),w.Object.create(c),proxy,revoked.proxy,null,undefined])assert(caught(()=>w.RTCDataChannel.prototype.close.call(receiver,extra)) instanceof w.TypeError);
   assert(state(w,c)==='connecting'&&reads===0&&conversions===0);
  });
  await run('PC shutdown from a channel close handler closes its remaining sibling',async pc=>{
   const c=pc.createDataChannel('first'),sibling=pc.createDataChannel('sibling'),completion=watch(w,c),other=watch(w,sibling);let closed;
   c.onclose=()=>{pc.close();closed=state(w,sibling);};c.close();await completion.wait();await tick();
   assert(closed==='closed'&&state(w,sibling)==='closed'&&completion.events.length===1&&other.events.length===0);
  });
 }
 for(const [name,callee,owner] of [['child callee',child,window],['main callee',window,child]]){
  await check(`${name}: borrowed channel close keeps the event in the channel realm`,async()=>{
   const pc=new owner.RTCPeerConnection(),c=pc.createDataChannel('foreign'),completion=watch(owner,c);
   try{
    for(const name of ['onopen','onclosing','onclose','onmessage','onerror','onbufferedamountlow']){
     const d=callee.Object.getOwnPropertyDescriptor(callee.RTCDataChannel.prototype,name),fn=()=>{};
     d.set.call(c,fn);assert(d.get.call(c)===fn);d.set.call(c,null);
     const e=caught(()=>d.set.call(new owner.Proxy(c,{}),fn));assert(e instanceof callee.TypeError&&!(e instanceof owner.TypeError));
    }
    const close=callee.RTCDataChannel.prototype.close;let conversions=0;close.call(c,{toString(){conversions++;throw Error('extra');}});
    assert(state(owner,c)==='closing'&&conversions===0);const e=await completion.wait();assert(e instanceof owner.Event&&!(e instanceof callee.Event)&&e.isTrusted);
    const error=caught(()=>close.call(new owner.Proxy(c,{})));assert(error instanceof callee.TypeError&&!(error instanceof owner.TypeError));
   }finally{pc.close();}
  });
  await check(`${name}: borrowed PC close reaches the receiver registry`,async()=>{
   const pc=new owner.RTCPeerConnection(),c=pc.createDataChannel('foreign PC'),completion=watch(owner,c);
   try{callee.RTCPeerConnection.prototype.close.call(pc);assert(state(owner,c)==='closed'&&pc.signalingState==='closed');await tick();assert(completion.events.length===0);}
   finally{pc.close();}
  });
 }
 globalThis.__uiEventResults={complete:true,passed:checks.filter(row=>row.passed).length,total:checks.length,checks};return checks.every(row=>row.passed);
})()
