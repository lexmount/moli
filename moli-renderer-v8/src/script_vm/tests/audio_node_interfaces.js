(async()=>{
 const rows=[],assert=(ok,message)=>{if(!ok)throw Error(message);},check=async(name,fn)=>{try{await fn();rows.push({name,pass:true});}catch(error){rows.push({name,pass:false,message:String(error)});}};
 const realms=[['main',window],['child',document.getElementById('child').contentWindow]];
 for(const [label,w] of realms){
  const context=new w.AudioContext(),prototype=w.AudioNode?.prototype;
  try {
   await check(label+'/AudioNode',()=>{
    const C=w.AudioNode,d=Object.getOwnPropertyDescriptor(w,'AudioNode');
    assert(typeof C==='function'&&C.name==='AudioNode'&&C.length===0,'constructor');
    assert(d.writable&&d.configurable&&!d.enumerable,'global descriptor');
    assert(Object.getPrototypeOf(C)===w.EventTarget&&Object.getPrototypeOf(C.prototype)===w.EventTarget.prototype,'native EventTarget inheritance');
    assert(C.prototype.constructor===C,'prototype constructor');
    for(const call of [()=>C(),()=>new C()]){let error;try{call();}catch(e){error=e;}assert(error instanceof w.TypeError,'illegal constructor in callee realm');}
   });
   const nodes=[['destination',context.destination,w.AudioDestinationNode,1,0,2,'explicit'],['oscillator',context.createOscillator(),w.OscillatorNode,0,1,2,'max'],['analyser',context.createAnalyser(),w.AnalyserNode,1,1,2,'max'],['compressor',context.createDynamicsCompressor(),w.DynamicsCompressorNode,1,1,2,'clamped-max'],['biquad',context.createBiquadFilter(),w.BiquadFilterNode,1,1,2,'max']];
   for(const [name,node,C,inputs,outputs,count,mode] of nodes){
    await check(label+'/'+name+'/inheritance',()=>{
     assert(node instanceof C&&node instanceof w.AudioNode&&node instanceof w.EventTarget,'native node identity');
     assert((name==='oscillator'?prototype.isPrototypeOf(C.prototype):Object.getPrototypeOf(C.prototype)===prototype),'native interface hierarchy');
     for(const key of ['context','connect','disconnect'])assert(!Object.hasOwn(node,key)&&!Object.hasOwn(C.prototype,key),'common property owner');
     assert(node.context===context&&node.numberOfInputs===inputs&&node.numberOfOutputs===outputs&&node.channelCount===count&&node.channelCountMode===mode&&node.channelInterpretation==='speakers','channel defaults');
    });
    await check(label+'/'+name+'/EventTarget',()=>{
     let calls=0;const listener=function(event){assert(this===node&&event.target===node,'event identity');calls++;};
     w.EventTarget.prototype.addEventListener.call(node,'test',listener);
     w.EventTarget.prototype.dispatchEvent.call(node,new w.Event('test'));
     w.EventTarget.prototype.removeEventListener.call(node,'test',listener);
     node.dispatchEvent(new w.Event('test'));assert(calls===1,'shared native listener storage');
    });
   }
   const real=nodes[2][1],entries=[['context','get'],['numberOfInputs','get'],['numberOfOutputs','get'],['channelCount','get'],['channelCount','set'],['channelCountMode','get'],['channelCountMode','set'],['channelInterpretation','get'],['channelInterpretation','set'],['connect','value'],['disconnect','value']];
   for(const [name,kind] of entries){await check(label+'/'+name+'/'+kind,()=>{
    const d=Object.getOwnPropertyDescriptor(prototype,name),fn=d[kind];
    assert(typeof fn==='function'&&d.enumerable&&d.configurable,'shared descriptor');
    assert(fn.name===(kind==='value'?'':kind+' ')+name&&fn.length===(kind==='set'||name==='connect'?1:0),'member name and length');
    let conversions=0,traps=0;const ignored={valueOf(){conversions++;throw Error('conversion');},toString(){conversions++;throw Error('conversion');}},trap=()=>{traps++;throw Error('author trap');},revoked=Proxy.revocable(real,{});revoked.revoke();
    for(const receiver of [null,{},prototype,Object.create(real),new Proxy(real,{get:trap,getPrototypeOf:trap}),revoked.proxy]){
     let error;try{fn.call(receiver,ignored,ignored,ignored);}catch(e){error=e;}assert(error instanceof w.TypeError,'callee realm rejects unbranded receiver');
    }
    assert(conversions===0&&traps===0,'brand precedes conversions and author hooks');
   });}
   await check(label+'/channels',()=>{
    const node=nodes[2][1],compressor=nodes[3][1];
    node.channelCount={valueOf(){return 3.75;}};node.channelCountMode='explicit';node.channelInterpretation='discrete';
    assert(node.channelCount===3&&node.channelCountMode==='explicit'&&node.channelInterpretation==='discrete','native channel writes');
    for(const [key,value,type] of [['channelCount',0,'NotSupportedError'],['channelCount',33,'NotSupportedError']]){
     let error;try{node[key]=value;}catch(e){error=e;}assert(error?.name===type,'invalid '+key);}
    for(const key of ['channelCountMode','channelInterpretation']){
     const before=node[key];node[key]='invalid';assert(node[key]===before,'ignore invalid enum attribute');
     const sentinel={};let error;try{node[key]={toString(){throw sentinel;}};}catch(e){error=e;}assert(error===sentinel,'preserve enum string conversion exception');
    }
    for(const [key,value] of [['channelCount',3],['channelCountMode','max']]){let error;try{compressor[key]=value;}catch(e){error=e;}assert(error?.name==='NotSupportedError','compressor limit');}
   });
   await check(label+'/cross-realm',()=>{
    const foreign=new window.AudioContext();try {
     const source=foreign.createOscillator(),target=foreign.createAnalyser();
     assert(Object.getOwnPropertyDescriptor(prototype,'context').get.call(source)===foreign,'genuine foreign node');
     assert(prototype.connect.call(source,target)===target,'borrowed native method');prototype.disconnect.call(source,target);
     Object.getOwnPropertyDescriptor(prototype,'channelCount').set.call(target,3);assert(target.channelCount===3,'borrowed native setter');
     let error;try{Object.getOwnPropertyDescriptor(prototype,'channelCountMode').set.call(target,Symbol());}catch(e){error=e;}
     assert(error instanceof w.TypeError,'conversion exception in callee realm');
    }finally{foreign.close();}
   });
   await check(label+'/graph',()=>{
    const source=nodes[1][1],target=nodes[2][1];
    assert(prototype.connect.call(source,target)===target,'connect returns target');prototype.disconnect.call(source,target);
    let error;try{prototype.disconnect.call(source,target);}catch(e){error=e;}assert(error?.name==='InvalidAccessError','missing edge');
    const sentinel={};let conversions=[];try{prototype.connect.call(source,target,{valueOf(){conversions.push('output');return 4;}},{valueOf(){conversions.push('input');throw sentinel;}});}catch(e){error=e;}
    assert(error===sentinel&&conversions.join()==='output,input','convert all ports before range validation');
    const other=new w.AudioContext();try{try{prototype.connect.call(source,other.destination,0,{valueOf(){throw sentinel;}});}catch(e){error=e;}assert(error===sentinel,'conversion precedes context check');}finally{other.close();}
    Object.setPrototypeOf(source,null);assert(Object.getOwnPropertyDescriptor(prototype,'context').get.call(source)===context,'native identity independent of prototype');
   });
  }finally{await context.close();}
 }
 globalThis.__nodeReplacementResults={rows,passed:rows.filter(r=>r.pass).length,total:rows.length,failures:rows.filter(r=>!r.pass)};return rows.every(r=>r.pass);
})()
