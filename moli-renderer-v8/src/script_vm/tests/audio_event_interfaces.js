(async()=>{
 const completionFlags=[],rows=[],assert=(ok,message)=>{if(!ok)throw Error(message);},check=async(name,fn)=>{try{await fn();rows.push({name,pass:true});}catch(error){rows.push({name,pass:false,message:String(error)});}},errorFrom=fn=>{try{fn();}catch(error){return error;}throw Error('expected exception');};
 const input=await new OfflineAudioContext(1,8,8000).startRendering(),output=await new OfflineAudioContext(1,16,8000).startRendering(),popup=open(),realms=[['main',window],['child',document.getElementById('child').contentWindow],['popup',popup]];
 try {for(const [label,w] of realms){
  const specs=[['AudioProcessingEvent',{inputBuffer:input,outputBuffer:output,playbackTime:1.25},['inputBuffer','outputBuffer','playbackTime']],['OfflineAudioCompletionEvent',{renderedBuffer:input},['renderedBuffer']]];
  for(const [name,payload,members] of specs){
   const init=()=>({...payload,bubbles:true,cancelable:true,composed:true});
   await check(label+'/'+name+'/interface',()=>{
    const C=w[name],d=Object.getOwnPropertyDescriptor(w,name);assert(typeof C==='function'&&C.name===name&&C.length===2,'constructor');
    assert(d.writable&&d.configurable&&!d.enumerable,'global descriptor');assert(Object.getPrototypeOf(C)===w.Event&&Object.getPrototypeOf(C.prototype)===w.Event.prototype&&C.prototype.constructor===C,'Event inheritance');
    assert(errorFrom(()=>C('test',init())) instanceof w.TypeError,'requires new');for(const args of [[],['test'],['test',null],['test',{}],['test',1]])assert(errorFrom(()=>new C(...args)) instanceof w.TypeError,'required dictionary and members');
   });
   await check(label+'/'+name+'/payload',()=>{
    const C=w[name],event=new C('payload\ud800',init());assert(event instanceof C&&event instanceof w.Event&&Object.prototype.toString.call(event)==='[object '+name+']','event brand');
    assert(event.type==='payload\ud800'&&event.bubbles&&event.cancelable&&event.composed&&!event.isTrusted,'EventInit and UTF-16');
    for(const key of members){assert(event[key]===payload[key]&&!Object.hasOwn(event,key),'native readonly payload '+key);Reflect.set(event,key,null);assert(event[key]===payload[key],'readonly setter');}
    class Sub extends C{};const sub=new Sub('test',init());assert(sub instanceof Sub&&sub instanceof C&&sub.type==='test','subclass');
    Object.freeze(event);for(const key of members)assert(event[key]===payload[key],'freeze preserves payload');
   });
   await check(label+'/'+name+'/dictionary conversion',()=>{
    const order=[],dictionary=Object.create(null);for(const key of ['bubbles','cancelable','composed',...members])Object.defineProperty(dictionary,key,{get(){order.push(key);return key in payload?payload[key]:true;}});
    const event=new w[name]({toString(){order.push('type');return 'test';}},dictionary);assert(order.join()==='type,bubbles,cancelable,composed,'+members.join(),'dictionary lexical order');
    assert(event.type==='test','converted type');
    const sentinel={};assert(errorFrom(()=>new w[name]({toString(){throw sentinel;}},new Proxy({},{get(){throw Error('dictionary prematurely read');}})))===sentinel,'type before dictionary');
    for(const key of ['bubbles','cancelable','composed',...members]){const bad=init();Object.defineProperty(bad,key,{get(){throw sentinel;}});assert(errorFrom(()=>new w[name]('test',bad))===sentinel,'preserve getter exception '+key);}
    for(const key of members){const missing=init();delete missing[key];assert(errorFrom(()=>new w[name]('test',missing)) instanceof w.TypeError,'missing required '+key);}
   });
   await check(label+'/'+name+'/buffer brands',()=>{
    for(const key of members.filter(k=>k.endsWith('Buffer'))){let traps=0;const revoked=Proxy.revocable(input,{});revoked.revoke();for(const buffer of [undefined,null,{},Object.create(input),new Proxy(input,{get(){traps++;throw Error('author proxy trap');}}),revoked.proxy])assert(errorFrom(()=>new w[name]('test',{...init(),[key]:buffer})) instanceof w.TypeError,'non-null native AudioBuffer required');assert(traps===0,'no author Proxy traps');}
   });
   await check(label+'/'+name+'/EventTarget dispatch',()=>{
    const event=new w[name]('test',init()),target=new w.EventTarget();let count=0;target.addEventListener('test',function(received){assert(received===event&&received.target===target&&received.currentTarget===target&&this===target&&received.eventPhase===w.Event.AT_TARGET,'native dispatch identity');count++;received.preventDefault();});
    assert(target.dispatchEvent(event)===false&&count===1&&event.defaultPrevented&&event.currentTarget===null&&!event.isTrusted,'Event dispatch and cancellation');for(const key of members)assert(event[key]===payload[key],'dispatch preserves payload');
   });
   for(const key of members)await check(label+'/'+name+'/'+key+'/receiver',()=>{
    const C=w[name],d=Object.getOwnPropertyDescriptor(C.prototype,key),fn=d.get;assert(d.enumerable&&d.configurable&&d.set===undefined&&fn.name==='get '+key&&fn.length===0,'readonly getter descriptor');
    const event=new C('test',init()),trap=()=>{throw Error('author trap');},revoked=Proxy.revocable(event,{});revoked.revoke();
    for(const receiver of [null,{},C.prototype,Object.create(event),new Proxy(event,{get:trap,getPrototypeOf:trap}),revoked.proxy])assert(errorFrom(()=>fn.call(receiver)) instanceof w.TypeError,'native receiver in callee realm');
    const foreign=new window[name]('test',init());assert(fn.call(foreign)===payload[key],'genuine foreign event');Object.setPrototypeOf(foreign,null);assert(fn.call(foreign)===payload[key],'brand independent of prototype');
   });
  }
  await check(label+'/AudioProcessingEvent/restricted playbackTime',()=>{
   for(const value of [NaN,Infinity,-Infinity,Symbol(),1n])assert(errorFrom(()=>new w.AudioProcessingEvent('test',{inputBuffer:input,outputBuffer:output,playbackTime:value})) instanceof w.TypeError,'restricted double');
   const event=new w.AudioProcessingEvent('test',{inputBuffer:input,outputBuffer:output,playbackTime:{valueOf(){return -1.5;}}});assert(event.playbackTime===-1.5,'negative finite payload preserved');
  });
  for(const mode of ['native','borrowed','replaced-global'])await check(label+'/actual offline completion/'+mode,async()=>{
   const C=w.OfflineAudioCompletionEvent,E=w.Event,context=new w.OfflineAudioContext(1,8,8000),observations={},completed=new Promise(resolve=>{context.addEventListener('complete',function(event){observations.listener={event,target:event.target,current:event.currentTarget,receiver:this,phase:event.eventPhase,trusted:event.isTrusted};});context.oncomplete=function(event){observations.handler={event,target:event.target,current:event.currentTarget,receiver:this};resolve();};});
   let rendered;try{
    if(mode==='replaced-global'){w.OfflineAudioCompletionEvent=function(){throw Error('author constructor');};w.Event=function(){throw Error('author Event');};}
    rendered=await (mode==='borrowed'?window.OfflineAudioContext.prototype.startRendering.call(context):context.startRendering());await completed;
   }finally{w.OfflineAudioCompletionEvent=C;w.Event=E;}
   const a=observations.listener,b=observations.handler,event=a.event;assert(event===b.event&&event instanceof C&&event instanceof E&&Object.getPrototypeOf(event)===C.prototype,'native completion event in context realm');
   assert(a.target===context&&a.current===context&&a.receiver===context&&a.phase===E.AT_TARGET&&a.trusted&&b.target===context&&b.current===context&&b.receiver===context,'trusted complete dispatch identity');
   completionFlags.push({realm:label,mode,bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed});
   assert(event.type==='complete'&&typeof event.bubbles==='boolean'&&!event.cancelable&&!event.composed&&event.currentTarget===null&&event.renderedBuffer===rendered,'complete payload and cleanup '+JSON.stringify({type:event.type,bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,currentIsNull:event.currentTarget===null,sameBuffer:event.renderedBuffer===rendered}));
  });
 }}finally{popup.close();}
 globalThis.__audioEventResults={completionFlags,rows,passed:rows.filter(r=>r.pass).length,total:rows.length,failures:rows.filter(r=>!r.pass)};return rows.every(r=>r.pass);
})()
