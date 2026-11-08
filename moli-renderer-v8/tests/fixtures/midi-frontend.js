(async () => {
  const checks=[];
  const assert=(ok,name)=>{if(!ok) throw Error(name)};
  const check=async(name,action)=>{try{await action();checks.push({name,passed:true})}catch(error){checks.push({name,passed:false,error:String(error)})}};
  const throws=(C,action)=>{let error;try{action()}catch(caught){error=caught}assert(error instanceof C,'Expected '+C.name+', got '+error)};
  for(const [index,w] of [globalThis,document.querySelector('iframe').contentWindow].entries()) {
    const prefix=index?'iframe ':'main ';
    for(const [name,parent,attributes,methods] of [
      ['MIDIAccess','EventTarget',['inputs','outputs','sysexEnabled','onstatechange'],[]],
      ['MIDIPort','EventTarget',['id','manufacturer','name','version','type','state','connection','onstatechange'],[['open',0],['close',0]]],
      ['MIDIInput','MIDIPort',['onmidimessage'],[]],
      ['MIDIOutput','MIDIPort',[],[['send',1],['clear',0]]],
      ['MIDIInputMap','Object',['size'],[['get',1],['has',1],['entries',0],['keys',0],['values',0],['forEach',1]]],
      ['MIDIOutputMap','Object',['size'],[['get',1],['has',1],['entries',0],['keys',0],['values',0],['forEach',1]]]
    ]) {
      await check(prefix+name+' native interface',()=>{
        const C=w[name]; assert(typeof C==='function'&&C.name===name&&C.length===0,'interface');
        assert(Object.getPrototypeOf(C.prototype)===w[parent].prototype,'inheritance');
        throws(w.TypeError,()=>new C()); throws(w.TypeError,()=>C());
      });
      for(const attribute of attributes) {
        await check(prefix+name+'.'+attribute+' descriptor',()=>{
          const d=Object.getOwnPropertyDescriptor(w[name].prototype,attribute);
          assert(d&&d.enumerable&&d.configurable&&typeof d.get==='function'&&d.get.length===0,'getter shape');
          if(attribute.startsWith('on')) assert(typeof d.set==='function'&&d.set.length===1,'handler setter');
          else assert(d.set===undefined,'readonly');
        });
        await check(prefix+name+'.'+attribute+' receiver',()=>{
          const P=w[name].prototype,d=Object.getOwnPropertyDescriptor(P,attribute);
          const revoked=Proxy.revocable(P,{});revoked.revoke();let traps=0;
          for(const value of [{},P,Object.create(P),new Proxy(P,{get(){traps++;throw 42},getPrototypeOf(){traps++;throw 43}}),revoked.proxy]) {
            throws(w.TypeError,()=>d.get.call(value));
            if(d.set) throws(w.TypeError,()=>d.set.call(value,{}));
          }
          assert(traps===0,'brand checking does not execute traps');
        });
      }
      for(const [method,length] of methods) {
        await check(prefix+name+'.'+method+' descriptor',()=>{
          const d=Object.getOwnPropertyDescriptor(w[name].prototype,method);
          assert(d&&d.enumerable&&d.configurable&&d.writable&&d.value.length===length&&d.value.name===method,'method shape');
        });
        await check(prefix+name+'.'+method+' receiver conversion order',async()=>{
          const P=w[name].prototype,revoked=Proxy.revocable(P,{});revoked.revoke();let reads=0,traps=0;
          const input={get [Symbol.iterator](){reads++;throw 44},toString(){reads++;throw 45}};
          for(const value of [{},P,Object.create(P),new Proxy(P,{get(){traps++;throw 46},getPrototypeOf(){traps++;throw 47}}),revoked.proxy]) {
            if(name==='MIDIPort') {
              const promise=P[method].call(value,input);assert(promise instanceof w.Promise,'callee Promise');
              let error;try{await promise}catch(caught){error=caught}assert(error instanceof w.TypeError,'callee rejection');
            } else throws(w.TypeError,()=>P[method].call(value,input));
          }
          assert(reads===0&&traps===0,'receiver before parameter conversion');
        });
      }
      if(name.endsWith('Map')) await check(prefix+name+' readonly iterator alias',()=>{
        const P=w[name].prototype,d=Object.getOwnPropertyDescriptor(P,Symbol.iterator);
        assert(d&&d.value===P.entries&&d.writable&&d.configurable&&!d.enumerable,'iterator alias');
        assert(!Object.hasOwn(P,'set')&&!Object.hasOwn(P,'delete')&&!Object.hasOwn(P,'clear'),'readonly surface');
      });
    }
    await check(prefix+'requestMIDIAccess secure descriptor',()=>{
      const d=Object.getOwnPropertyDescriptor(w.Navigator.prototype,'requestMIDIAccess');
      assert(w.isSecureContext&&d&&d.value.length===0&&d.value.name==='requestMIDIAccess'&&d.enumerable&&d.configurable&&d.writable,'secure request surface');
    });
    await check(prefix+'requestMIDIAccess dictionary order and unsupported backend',async()=>{
      const log=[],options={get software(){log.push('software');return {valueOf(){throw Error('ToBoolean')}}},get sysex(){log.push('sysex');return true}};
      const promise=w.Navigator.prototype.requestMIDIAccess.call(navigator,options);
      assert(promise instanceof w.Promise&&log.join()==='software,sysex','synchronous dictionary conversion, callee Promise');
      let error;try{await promise}catch(caught){error=caught}
      assert(error instanceof w.DOMException&&error.name==='NotSupportedError','explicit unsupported platform');
    });
    for(const [optionIndex,options] of [undefined,null,{},Object.create(null)].entries()) await check(prefix+'requestMIDIAccess empty dictionary '+optionIndex,async()=>{
      let error;try{await w.navigator.requestMIDIAccess(options)}catch(caught){error=caught}
      assert(error instanceof w.DOMException&&error.name==='NotSupportedError','no invented device access');
    });
    for(const options of [1,true,'midi',Symbol(),1n]) await check(prefix+'requestMIDIAccess invalid dictionary '+String(options),async()=>{
      let error;try{await w.navigator.requestMIDIAccess(options)}catch(caught){error=caught}
      assert(error instanceof w.TypeError,'invalid dictionary rejection');
    });
    await check(prefix+'requestMIDIAccess getter exception identity',async()=>{
      const marker={};let sysexReads=0,error;
      const promise=w.navigator.requestMIDIAccess({get software(){throw marker},get sysex(){sysexReads++;return false}});
      assert(promise instanceof w.Promise,'conversion failure returns Promise');
      try{await promise}catch(caught){error=caught}
      assert(error===marker&&sysexReads===0,'abrupt dictionary member stops next getter');
    });
    await check(prefix+'requestMIDIAccess native receiver before conversion',async()=>{
      const real=w.navigator,revoked=Proxy.revocable(real,{});revoked.revoke();let reads=0,traps=0;
      const options={get software(){reads++;throw 51},get sysex(){reads++;throw 52}};
      for(const value of [{},w.Navigator.prototype,Object.create(real),new Proxy(real,{get(){traps++;throw 53},getPrototypeOf(){traps++;throw 54}}),revoked.proxy]) {
        const promise=w.Navigator.prototype.requestMIDIAccess.call(value,options);assert(promise instanceof w.Promise,'receiver rejection Promise');
        let error;try{await promise}catch(caught){error=caught}assert(error instanceof w.TypeError,'callee receiver error');
      }
      assert(reads===0&&traps===0,'private receiver branding');
    });
  }
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  return checks.every(row=>row.passed);
})()
