(async()=>{
 const rows=[],assert=(ok,message)=>{if(!ok)throw Error(message);},check=async(name,fn)=>{try{await fn();rows.push({name,pass:true});}catch(error){rows.push({name,pass:false,message:String(error)});}},errorFrom=fn=>{try{fn();}catch(error){return error;}throw Error('expected exception');};
 const popup=open(),realms=[['main',window],['child',document.getElementById('child').contentWindow],['popup',popup]];
 try {for(const [label,w] of realms){
  const context=new w.AudioContext(),scheduled=w.AudioScheduledSourceNode?.prototype;
  try {
   for(const [name,parent,length] of [['AudioScheduledSourceNode','AudioNode',0],['AudioBufferSourceNode','AudioScheduledSourceNode',1],['ConstantSourceNode','AudioScheduledSourceNode',1]]){
    await check(label+'/'+name+'/interface',()=>{
     const C=w[name],P=w[parent],d=Object.getOwnPropertyDescriptor(w,name);
     assert(typeof C==='function'&&C.name===name&&C.length===length,'native constructor name and length');
     assert(d.writable&&d.configurable&&!d.enumerable,'global descriptor');
     assert(Object.getPrototypeOf(C)===P&&Object.getPrototypeOf(C.prototype)===P.prototype&&C.prototype.constructor===C,'native interface inheritance');
     assert(errorFrom(()=>C(context)) instanceof w.TypeError,'requires new');
     if(name==='AudioScheduledSourceNode')assert(errorFrom(()=>new C()) instanceof w.TypeError,'abstract interface');
    });
   }
   await check(label+'/OscillatorNode/inheritance',()=>{
    const node=context.createOscillator();assert(node instanceof w.AudioScheduledSourceNode&&Object.getPrototypeOf(w.OscillatorNode.prototype)===scheduled,'shared scheduled source');
    assert(!Object.hasOwn(node,'start')&&!Object.hasOwn(w.OscillatorNode.prototype,'start')&&node.start===scheduled.start,'shared start owner');
   });
   const factories=[['AudioBufferSourceNode','createBufferSource'],['ConstantSourceNode','createConstantSource']];
   for(const [name,factory] of factories){
    await check(label+'/'+name+'/constructor and factory',()=>{
     const C=w[name],nodes=[new C(context),new C(context,null),context[factory]()];
     for(const node of nodes){
      assert(Object.getPrototypeOf(node)===C.prototype&&node instanceof w.AudioNode&&node instanceof w.EventTarget,'native source identity');
      assert(node.context===context&&node.numberOfInputs===0&&node.numberOfOutputs===1&&node.channelCount===2&&node.channelCountMode==='max'&&node.channelInterpretation==='speakers','native source defaults');
      assert(!Object.hasOwn(node,'start')&&!Object.hasOwn(node,'stop')&&!Object.hasOwn(node,'onended'),'prototype members');
      if(name==='ConstantSourceNode')assert(node.offset instanceof w.AudioParam&&node.offset===node.offset&&node.offset.value===1&&node.offset.defaultValue===1&&node.offset.automationRate==='a-rate','offset AudioParam');
      else assert(node.buffer===null&&node.loop===false&&node.loopStart===0&&node.loopEnd===0&&node.playbackRate.value===1&&node.playbackRate.defaultValue===1&&node.detune.value===0&&node.detune.defaultValue===0&&node.detune.automationRate==='k-rate'&&node.playbackRate.automationRate==='k-rate','buffer source defaults');
     }
    });
    await check(label+'/'+name+'/constructor context validation',()=>{
     const C=w[name];let reads=0,traps=0;const options=new Proxy({},{get(){reads++;throw Error('options getter');}}),revoked=Proxy.revocable(context,{});revoked.revoke();
     for(const value of [undefined,null,{},Object.create(context),new Proxy(context,{get(){traps++;throw Error('author trap');}}),revoked.proxy])assert(errorFrom(()=>new C(value,options)) instanceof w.TypeError,'reject forged context');
     assert(reads===0&&traps===0,'context brand before options conversion');
     for(const value of [1,Symbol(),true,'options'])assert(errorFrom(()=>new C(context,value)) instanceof w.TypeError,'dictionary object required');
     class Sub extends C{};const sub=new Sub(context);assert(sub instanceof Sub&&sub instanceof C&&sub.context===context,'subclass constructor preserves prototype');
    });
    await check(label+'/'+name+'/start stop state and conversion',()=>{
     const node=context[factory]();
     for(const method of ['start','stop'])for(const value of [NaN,Infinity,-Infinity,Symbol(),1n])assert(errorFrom(()=>node[method](value)) instanceof w.TypeError,'restricted double '+method);
     assert(errorFrom(()=>node.stop(-1))?.name==='InvalidStateError','state before negative stop constraint');
     assert(errorFrom(()=>node.stop())?.name==='InvalidStateError','stop before start');
     assert(errorFrom(()=>node.start(-1)) instanceof w.RangeError,'negative start does not start node');
     assert(node.start(100)===undefined,'successful start');
     assert(errorFrom(()=>node.start(-1))?.name==='InvalidStateError','repeated start state before range');
     assert(errorFrom(()=>node.start(NaN)) instanceof w.TypeError,'double conversion before repeated start state');
     assert(errorFrom(()=>node.stop(-1)) instanceof w.RangeError,'negative stop');
     assert(node.stop(102)===undefined&&node.stop(101)===undefined,'last valid stop can replace earlier stop');
    });
   }
   await check(label+'/OscillatorNode/start stop state',()=>{
    const node=context.createOscillator();assert(errorFrom(()=>node.stop(-1))?.name==='InvalidStateError','stop before start');
    assert(errorFrom(()=>node.start(-1)) instanceof w.RangeError,'negative start');node.start(100);
    assert(errorFrom(()=>node.start(-1))?.name==='InvalidStateError','repeated start');assert(errorFrom(()=>node.start(Infinity)) instanceof w.TypeError,'conversion before state');node.stop(101);
   });
   await check(label+'/options conversion',()=>{
    const order=[],options=Object.create({get loop(){order.push('loop');return new Boolean(false);}});
    for(const [key,value] of [['buffer',null],['detune',.2],['loopEnd',-3],['loopStart',-2],['playbackRate',.7]])Object.defineProperty(options,key,{get(){order.push(key);return value;}});
    const node=new w.AudioBufferSourceNode(context,options);
    assert(order.join()==='buffer,detune,loop,loopEnd,loopStart,playbackRate','lexical dictionary evaluation order');
    assert(node.loop&&node.loopStart===-2&&node.loopEnd===-3&&node.detune.value===Math.fround(.2)&&node.playbackRate.value===Math.fround(.7)&&node.detune.defaultValue===0&&node.playbackRate.defaultValue===1,'options preserve default metadata');
    const constant=new w.ConstantSourceNode(context,{offset:.1,get channelCount(){throw Error('unused AudioNodeOptions');}});
    assert(constant.offset.value===Math.fround(.1)&&constant.offset.defaultValue===1,'float rounding and correct dictionary members');
    for(const C of [w.AudioBufferSourceNode,w.ConstantSourceNode]){const sentinel={};assert(errorFrom(()=>new C(context,new Proxy({},{get(){throw sentinel;}})))===sentinel,'preserve dictionary getter exception');}
    for(const value of [Infinity,NaN,1e100,Symbol(),1n]){assert(errorFrom(()=>new w.ConstantSourceNode(context,{offset:value})) instanceof w.TypeError,'finite float option');assert(errorFrom(()=>new w.AudioBufferSourceNode(context,{playbackRate:value})) instanceof w.TypeError,'finite float playback rate');}
   });
   await check(label+'/source parameter metadata and automation rates',()=>{
    const constant=context.createConstantSource(),buffer=context.createBufferSource(),limit=Math.fround(3.4028234663852886e38);
    for(const param of [constant.offset,buffer.detune,buffer.playbackRate])assert(param.minValue===-limit&&param.maxValue===limit,'full float nominal range');
    constant.offset.automationRate='k-rate';assert(constant.offset.automationRate==='k-rate','constant offset rate can change');constant.offset.automationRate='a-rate';
    for(const param of [buffer.detune,buffer.playbackRate]){assert(errorFrom(()=>{param.automationRate='a-rate';})?.name==='InvalidStateError','buffer parameters fixed at k-rate');assert(param.automationRate==='k-rate','failed write preserves rate');param.automationRate='invalid';assert(param.automationRate==='k-rate','invalid enum attribute ignored');}
    for(const [node,key] of [[constant,'offset'],[buffer,'detune'],[buffer,'playbackRate']]){const before=node[key];Reflect.set(node,key,{});assert(node[key]===before&&!Object.hasOwn(node,key),'readonly native AudioParam accessor');}
   });
   await check(label+'/buffer source property conversion',()=>{
    const node=context.createBufferSource();node.loop={};node.loopStart={valueOf(){return -1.25;}};node.loopEnd='2.5';assert(node.loop&&node.loopStart===-1.25&&node.loopEnd===2.5,'boolean and double setters');
    for(const key of ['loopStart','loopEnd']){const before=node[key];for(const value of [NaN,Infinity,-Infinity,Symbol(),1n])assert(errorFrom(()=>{node[key]=value;}) instanceof w.TypeError,'finite double setter');assert(node[key]===before,'failed conversion preserves slot');}
    node.buffer=undefined;assert(node.buffer===null,'nullable interface maps undefined to null');
   });
   await check(label+'/buffer source start arguments',()=>{
    for(const values of [[0,-1],[0,0,-1]])assert(errorFrom(()=>context.createBufferSource().start(...values)) instanceof w.RangeError,'negative offset/duration');
    for(const values of [[0,Infinity],[0,0,Infinity],[0,undefined,NaN]])assert(errorFrom(()=>context.createBufferSource().start(...values)) instanceof w.TypeError,'restricted double arguments');
    const node=context.createBufferSource(),sentinel={};let order=[];
    assert(errorFrom(()=>node.start(-1,{valueOf(){order.push('offset');return 0;}},{valueOf(){order.push('duration');throw sentinel;}}))===sentinel&&order.join()==='offset,duration','all conversions before state and range validation');
    node.start(100,undefined,undefined);order=[];assert(errorFrom(()=>node.start(-1,{valueOf(){order.push('offset');throw sentinel;}}))===sentinel&&order.join()==='offset','conversion before repeated start');
   });
   const sourceEntries=[['AudioScheduledSourceNode','start','value'],['AudioScheduledSourceNode','stop','value'],['AudioScheduledSourceNode','onended','get'],['AudioScheduledSourceNode','onended','set'],['AudioBufferSourceNode','buffer','get'],['AudioBufferSourceNode','buffer','set'],['AudioBufferSourceNode','loop','get'],['AudioBufferSourceNode','loop','set'],['AudioBufferSourceNode','loopStart','get'],['AudioBufferSourceNode','loopStart','set'],['AudioBufferSourceNode','loopEnd','get'],['AudioBufferSourceNode','loopEnd','set'],['AudioBufferSourceNode','playbackRate','get'],['AudioBufferSourceNode','detune','get'],['AudioBufferSourceNode','start','value'],['ConstantSourceNode','offset','get']];
   for(const [name,key,kind] of sourceEntries){await check(label+'/'+name+'/'+key+'/'+kind+'/receiver',()=>{
    const proto=w[name].prototype,d=Object.getOwnPropertyDescriptor(proto,key),fn=d[kind];assert(typeof fn==='function'&&d.enumerable&&d.configurable,'native shared descriptor');
    assert(fn.length===(kind==='set'?1:0)&&fn.name===(kind==='value'?'':kind+' ')+key,'member name and length');
    const real=context.createBufferSource();let conversions=0,traps=0;const value={valueOf(){conversions++;throw Error('conversion');},toString(){conversions++;throw Error('conversion');}},trap=()=>{traps++;throw Error('proxy trap');},revoked=Proxy.revocable(real,{});revoked.revoke();
    for(const receiver of [null,{},proto,Object.create(real),new Proxy(real,{get:trap,getPrototypeOf:trap}),revoked.proxy])assert(errorFrom(()=>fn.call(receiver,value,value,value)) instanceof w.TypeError,'reject unbranded receiver in callee realm');
    assert(conversions===0&&traps===0,'receiver validation precedes author hooks');
   });}
   await check(label+'/onended event handler ordering',()=>{
    const node=context.createConstantSource(),order=[],first=()=>order.push('first'),old=()=>order.push('old'),replacement=function(event){assert(this===node&&event.target===node&&event.currentTarget===node,'handler identity');order.push('handler');},last=()=>order.push('last');
    assert(node.onended===null,'initial null');node.addEventListener('ended',first);node.onended=old;node.addEventListener('ended',last);node.onended=replacement;assert(node.onended===replacement,'replace handler');
    node.dispatchEvent(new w.Event('ended'));assert(order.join()==='first,handler,last','replacement preserves registration order');
    node.onended=1;assert(node.onended===null,'primitive clears callback');order.length=0;node.dispatchEvent(new w.Event('ended'));assert(order.join()==='first,last','clear handler');
    node.onended=replacement;order.length=0;node.dispatchEvent(new w.Event('ended'));assert(order.join()==='first,last,handler','reactivation appends handler');
   });
   await check(label+'/buffer assignment branding and once-only state',async()=>{
    const buffer=await new w.OfflineAudioContext(1,8,8000).startRendering(),node=context.createBufferSource(),revoked=Proxy.revocable(buffer,{});revoked.revoke();let traps=0;
    for(const value of [{},Object.create(buffer),new Proxy(buffer,{get(){traps++;throw Error('proxy trap');}}),revoked.proxy])assert(errorFrom(()=>{node.buffer=value;}) instanceof w.TypeError,'AudioBuffer brand validation');assert(traps===0,'no buffer author proxy traps');
    node.buffer=null;node.buffer=buffer;assert(node.buffer===buffer,'genuine buffer');assert(errorFrom(()=>{node.buffer=buffer;})?.name==='InvalidStateError','second non-null assignment');node.buffer=null;assert(node.buffer===null,'clear buffer');assert(errorFrom(()=>{node.buffer=buffer;})?.name==='InvalidStateError','null does not reset buffer-set state');
    const other=new w.AudioBufferSourceNode(context,{buffer});assert(other.buffer===buffer&&errorFrom(()=>{other.buffer=buffer;})?.name==='InvalidStateError','constructor initializes buffer-set state');
   });
   await check(label+'/cross realm identity and errors',()=>{
    const foreign=new window.AudioContext();try{
     const node=new w.ConstantSourceNode(foreign);assert(node.context===foreign&&node instanceof w.ConstantSourceNode&&node.offset instanceof w.AudioParam,'constructor realm differs from context realm');
     const source=foreign.createBufferSource();assert(Object.getOwnPropertyDescriptor(scheduled,'onended').get.call(source)===null,'borrowed source getter accepts genuine foreign source');scheduled.start.call(source,100);scheduled.stop.call(source,101);
     assert(errorFrom(()=>scheduled.stop.call(source,Symbol())) instanceof w.TypeError,'conversion exception uses callee realm');
     Object.setPrototypeOf(source,null);assert(Object.getOwnPropertyDescriptor(w.AudioBufferSourceNode.prototype,'loop').get.call(source)===false,'native identity survives changed prototype');
    }finally{foreign.close();}
   });
  }finally{await context.close();}
 }}finally{popup.close();}
 globalThis.__nodeReplacementResults={rows,passed:rows.filter(r=>r.pass).length,total:rows.length,failures:rows.filter(r=>!r.pass)};return rows.every(r=>r.pass);
})()
