globalThis.__mousePointerProbe = realms => {
  const rows = [], errors = [], unexecuted = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const run = (label, work) => { try { work(); } catch (error) { errors.push({label, error: String(error.stack || error)}); } };
  const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const units = s => Array.from({length:s.length}, (_,i)=>s.charCodeAt(i));
  const integer = (n,bits,signed) => { n=Number(n); if(!Number.isFinite(n)||n===0)return 0; const m=2**bits; n=((Math.trunc(n)%m)+m)%m; return signed && n>=m/2?n-m:n; };
  const base = ['bubbles','cancelable','composed','detail','view','which'];
  const modifiers = ['altKey','ctrlKey','metaKey','modifierAltGraph','modifierCapsLock','modifierFn','modifierFnLock','modifierHyper','modifierNumLock','modifierScrollLock','modifierSuper','modifierSymbol','modifierSymbolLock','shiftKey'];
  const mouse = ['button','buttons','clientX','clientY','movementX','movementY','relatedTarget','screenX','screenY'];
  const pointer = ['altitudeAngle','azimuthAngle','coalescedEvents','height','isPrimary','persistentDeviceId','pointerId','pointerType','predictedEvents','pressure','tangentialPressure','tiltX','tiltY','twist','width'];
  const wheel = ['deltaMode','deltaX','deltaY','deltaZ'];
  const mouseFields = ['screenX','screenY','clientX','clientY','button','buttons','ctrlKey','altKey','shiftKey','metaKey','movementX','movementY','pageX','pageY','x','y','relatedTarget'];
  const pointerFields = pointer.filter(k=>!k.endsWith('Events'));
  for (const [r,realm] of realms.entries()) for (const name of ['MouseEvent','WheelEvent','PointerEvent','DragEvent']) {
    const C=realm[name], prefix=`${r}/${name}`, own=name==='MouseEvent'?mouseFields:name==='WheelEvent'?wheel:name==='PointerEvent'?pointerFields:['dataTransfer'];
    const keys=[...base,...modifiers,...mouse,...(name==='WheelEvent'?wheel:name==='PointerEvent'?pointer:name==='DragEvent'?['dataTransfer']:[])];
    run(prefix+'/metadata',()=>{
      const e=new C('x');
      record(prefix+'/constructor',{name:C.name===name,length:C.length===1,parent:Object.getPrototypeOf(C.prototype)===(name==='MouseEvent'?realm.UIEvent:realm.MouseEvent).prototype,instance:e instanceof realm.MouseEvent});
      for(const field of own) {
        const d=Object.getOwnPropertyDescriptor(C.prototype,field);
        record(prefix+'/descriptor-'+field,{getter:typeof d?.get==='function',name:d?.get?.name==='get '+field,length:d?.get?.length===0,enumerable:d?.enumerable===true,configurable:d?.configurable===true,setter:d?.set===undefined,own:!Object.hasOwn(e,field)});
      }
    });
    for(const [i,init] of [undefined,null,{},[],function(){},new Date(0)].entries())run(prefix+'/defaults-'+i,()=>{
      const e=new C('x',init);
      record(prefix+'/defaults-'+i,{flags:!e.bubbles&&!e.cancelable&&!e.composed,ui:e.view===null&&e.detail===0&&e.which===0,mouse:mouseFields.filter(k=>k!=='relatedTarget').every(k=>e[k]===(k.endsWith('Key')?false:0)),related:e.relatedTarget===null,derived:name==='WheelEvent'?wheel.every(k=>e[k]===0):name==='PointerEvent'?e.pointerId===0&&e.width===1&&e.height===1&&e.pressure===0&&e.tangentialPressure===0&&e.tiltX===0&&e.tiltY===0&&e.twist===0&&e.altitudeAngle===Math.PI/2&&e.azimuthAngle===0&&e.pointerType===''&&!e.isPrimary&&e.persistentDeviceId===0:name==='DragEvent'?e.dataTransfer===null:true});
    });
    for(const [i,init] of [0,1,'',true,Symbol('init'),1n].entries())run(prefix+'/dictionary-'+i,()=>{
      let error;try{new C('x',init);}catch(e){error=e;}record(prefix+'/dictionary-'+i,{typeError:error instanceof realm.TypeError},error?.name||'returned');
    });
    run(prefix+'/order',()=>{
      const trace=[];new C('x',new Proxy({}, {get(_,key){trace.push(key);}}));record(prefix+'/order',{order:eq(trace,keys)},trace);
      for(const [i,key] of keys.entries()){
        const sentinel={},reads=[];let error;try{new C('x',new Proxy({}, {get(_,k){reads.push(k);if(k===key)throw sentinel;}}));}catch(e){error=e;}
        record(prefix+'/throw-get-'+key,{identity:error===sentinel,order:eq(reads,keys.slice(0,i+1))},reads);
      }
      const trace2=[];const type={[Symbol.toPrimitive](hint){trace2.push('type:'+hint);return '\uD800';}};
      const e=new C(type,new Proxy({}, {get(_,key){trace2.push(key);}}));record(prefix+'/type-first',{type:e.type==='\uD800',order:eq(trace2,['type:string',...keys])},trace2);
      const sentinel={},reads=[];let error;try{new C({[Symbol.toPrimitive](){throw sentinel;}},new Proxy({}, {get(_,k){reads.push(k);}}));}catch(e){error=e;}record(prefix+'/type-throw',{identity:error===sentinel,noReads:reads.length===0});
    });
    const numeric={detail:['int',32,true,0],which:['int',32,false,0],button:['int',16,true,0],buttons:['int',16,false,0]};
    for(const k of ['screenX','screenY','clientX','clientY','movementX','movementY'])numeric[k]=['double',0,false,0];
    if(name==='WheelEvent'){numeric.deltaMode=['int',32,false,0];for(const k of ['deltaX','deltaY','deltaZ'])numeric[k]=['double',0,false,0];}
    if(name==='PointerEvent'){
      for(const k of ['pointerId','persistentDeviceId','tiltX','tiltY','twist'])numeric[k]=['int',32,true,0];
      for(const k of ['width','height'])numeric[k]=['double',0,false,1];
      for(const k of ['pressure','tangentialPressure'])numeric[k]=['float',0,false,0];
      numeric.altitudeAngle=['double',0,false,Math.PI/2];numeric.azimuthAngle=['double',0,false,0];
    }
    const values=[undefined,null,0,-0,-1,-3.5,3.5,65535,65536,2147483648,4294967295,4294967297,NaN,Infinity,-Infinity,true,false,'3.5','NaN',1e40];
    for(const [field,[kind,bits,signed,def]] of Object.entries(numeric)){
      for(const [i,value] of (['altitudeAngle','azimuthAngle'].includes(field)?[undefined,null,0,-0,-1,-3.5,3.5,NaN,Infinity,-Infinity,true,false,'3.5','NaN',Math.PI/4]:values).entries())run(prefix+'/number-'+field+'-'+i,()=>{
        const n=value===undefined?def:Number(value),expected=kind==='int'?integer(n,bits,signed):kind==='float'?Math.fround(n):n;
        const valid=kind==='int'||Number.isFinite(expected);let e,error;try{e=new C('x',{[field]:value});}catch(caught){error=caught;}
        record(prefix+'/number-'+field+'-'+i,valid?{value:Object.is(e?.[field],expected)}:{typeError:error instanceof realm.TypeError},{actual:e?.[field],expected:String(expected),error:error?.name});
      });
      for(const [i,value] of [Symbol('n'),1n].entries())run(prefix+'/invalid-number-'+field+'-'+i,()=>{let error;try{new C('x',{[field]:value});}catch(e){error=e;}record(prefix+'/invalid-number-'+field+'-'+i,{typeError:error instanceof realm.TypeError},error?.name||'returned');});
      run(prefix+'/number-hint-'+field,()=>{
        const trace=[],v={[Symbol.toPrimitive](hint){trace.push(hint);return 3.5;}};const e=new C('x',{[field]:v});
        record(prefix+'/number-hint-'+field,{hint:eq(trace,['number']),value:e[field]===(kind==='int'?3:3.5)},trace);
        const sentinel={},reads=[];let error;try{new C('x',new Proxy({}, {get(_,key){reads.push(key);return key===field?{[Symbol.toPrimitive](){throw sentinel;}}:undefined;}}));}catch(e){error=e;}
        record(prefix+'/throw-convert-'+field,{identity:error===sentinel,order:eq(reads,keys.slice(0,keys.indexOf(field)+1))},reads);
      });
    }
    for(const [i,key] of modifiers.entries())run(prefix+'/modifier-'+key,()=>{
      const e=new C('x',{[key]:{}}),names=['Alt','Control','Meta','AltGraph','CapsLock','Fn','FnLock','Hyper','NumLock','ScrollLock','Super','Symbol','SymbolLock','Shift'];
      record(prefix+'/modifier-'+key,{state:e.getModifierState(names[i]),others:names.filter((_,j)=>i!==j).every(k=>!e.getModifierState(k))});
    });
    for(const [i,value] of [null,undefined,realm.document,realm.document.createElement('div'),new realm.EventTarget(),realms[(r+1)%realms.length].document,realm.document.implementation.createHTMLDocument('').createElement('select')].entries())run(prefix+'/related-valid-'+i,()=>{
      const e=new C('x',{relatedTarget:value});record(prefix+'/related-valid-'+i,{identity:e.relatedTarget===(value??null)});
    });
    const bad=[{},Object.create(realm.EventTarget.prototype),Object.create(realm.document),new Proxy(realm.document,{}),new Proxy(new realm.EventTarget(),{})];
    const revoked=Proxy.revocable(realm.document,{});revoked.revoke();bad.push(revoked.proxy,1,'x',Symbol('bad'));
    for(const [i,value] of bad.entries())run(prefix+'/related-invalid-'+i,()=>{let error;try{new C('x',{relatedTarget:value});}catch(e){error=e;}record(prefix+'/related-invalid-'+i,{typeError:error instanceof realm.TypeError},error?.name||'returned');});
    run(prefix+'/related-no-traps',()=>{let traps=0,error;const p=new Proxy(realm.document,{get(){traps++;},getPrototypeOf(){traps++;}});try{new C('x',{relatedTarget:p});}catch(e){error=e;}record(prefix+'/related-no-traps',{typeError:error instanceof realm.TypeError,noTraps:traps===0},traps);});
    run(prefix+'/flags-aliases',()=>{
      for(let n=0;n<8;n++){
        const e=new C('x',{bubbles:!!(n&1),cancelable:!!(n&2),composed:!!(n&4),clientX:31,clientY:42,view:realm});
        record(prefix+'/flags-'+n,{bubbles:e.bubbles===!!(n&1),cancelable:e.cancelable===!!(n&2),composed:e.composed===!!(n&4),view:e.view===realm,aliases:e.x===31&&e.pageX===31&&e.y===42&&e.pageY===42&&e.offsetX===31&&e.offsetY===42});
      }
      const e=new C('x');realm.document.body.dispatchEvent(e);record(prefix+'/dispatch',{target:e.target===realm.document.body,phase:e.eventPhase===0,untrusted:!e.isTrusted});
    });
    for(const field of own)run(prefix+'/receiver-'+field,()=>{
      const getter=Object.getOwnPropertyDescriptor(C.prototype,field)?.get;
      if(typeof getter!=='function'){
        unexecuted.push({label:prefix+'/receiver-'+field,checks:12,reason:'missing prototype getter'});return;
      }
      const e=new C('x'),other=new realms[(r+1)%realms.length][name]('x');
      record(prefix+'/genuine-'+field,{same:Object.is(getter.call(e),e[field]),other:Object.is(getter.call(other),other[field])});
      const bad=[{},Object.create(C.prototype),Object.create(e),new Proxy(e,{})];let revoked=Proxy.revocable(e,{});revoked.revoke();bad.push(revoked.proxy);
      for(const [i,receiver] of bad.entries()){let error;try{getter.call(receiver);}catch(e){error=e;}record(prefix+'/receiver-'+field+'-'+i,{typeError:error instanceof realm.TypeError,noWrongRealm:r===0||!(error instanceof realms[0].TypeError)},error?.name||'returned');}
    });
    if(name==='PointerEvent')run(prefix+'/pointer',()=>{
      for(const [i,value] of [undefined,null,'pen','\uD800','\uDC00','x\uD800y','\uD83D\uDE80',1n,true,['a','b']].entries()){
        const actual=new C('x',{pointerType:value}).pointerType;record(prefix+'/pointerType-'+i,{value:actual===(value===undefined?'':String(value))},units(actual));
      }
      const trace=[],input={[Symbol.toPrimitive](hint){trace.push(hint);return '\uD800';}},actual=new C('x',{pointerType:input}).pointerType;record(prefix+'/pointerType-hint',{hint:eq(trace,['string']),value:actual==='\uD800'},units(actual));
      let error;try{new C('x',{pointerType:Symbol('x')});}catch(e){error=e;}record(prefix+'/pointerType-symbol',{typeError:error instanceof realm.TypeError});
      for(const key of ['coalescedEvents','predictedEvents']){
        const a=new C('a'),b=new realms[(r+1)%realms.length].PointerEvent('b'),list=[a,b],e=new C('x',{[key]:list}),method=key==='coalescedEvents'?'getCoalescedEvents':'getPredictedEvents';
        if(typeof e[method]!=='function' && !realm.isSecureContext && method==='getCoalescedEvents'){record(prefix+'/'+key+'-secure',{absent:true});continue;} if(typeof e[method]!=='function'){unexecuted.push({label:prefix+'/'+key+'-copy',checks:5,reason:'missing method in secure context'});continue;}
        list.length=0;const first=e[method](),second=e[method]();first.pop();record(prefix+'/'+key+'-copy',{copied:second.length===2,fresh:first!==second,identity:second[0]===a&&second[1]===b,stable:e[method]().length===2,array:second instanceof realm.Array});
        for(const [i,value] of [null,1,{},[{}],[new realm.MouseEvent('x')],[new Proxy(a,{})],[Object.create(a)],[Object.create(C.prototype)]].entries()){
          let error;try{new C('x',{[key]:value});}catch(e){error=e;}record(prefix+'/'+key+'-invalid-'+i,{typeError:error instanceof realm.TypeError},error?.name||'returned');
        }
        const sentinel={},trace=[];let error;try{new C('x',{[key]:{[Symbol.iterator](){trace.push('iterator');return{next(){trace.push('next');throw sentinel;}};}}});}catch(e){error=e;}record(prefix+'/'+key+'-iterator-throw',{identity:error===sentinel,trace:eq(trace,['iterator','next'])},trace);
        const fn=C.prototype[method];for(const [i,receiver] of [{},new realm.MouseEvent('x'),new Proxy(e,{})].entries()){let error;try{fn.call(receiver);}catch(e){error=e;}record(prefix+'/'+method+'-receiver-'+i,{typeError:error instanceof realm.TypeError});}
      }
      const supplied=new C('x',{tiltX:45,azimuthAngle:Math.PI/4}),derived=new C('x',{tiltY:45});record(prefix+'/angle-presence',{retained:supplied.tiltX===45&&supplied.azimuthAngle===Math.PI/4&&supplied.altitudeAngle===Math.PI/2,derived:derived.tiltX===0&&derived.tiltY===45&&derived.azimuthAngle===Math.PI/2&&Math.abs(derived.altitudeAngle-Math.PI/4)<1e-14});
    });
    if(name==='DragEvent')run(prefix+'/drag',()=>{
      const dt=new realm.DataTransfer(),e=new C('x',{dataTransfer:dt});record(prefix+'/dataTransfer-native',{identity:e.dataTransfer===dt});
      for(const [i,value] of [{},Object.create(realm.DataTransfer.prototype),Object.create(dt),new Proxy(dt,{})].entries()){let error;try{new C('x',{dataTransfer:value});}catch(e){error=e;}record(prefix+'/dataTransfer-invalid-'+i,{typeError:error instanceof realm.TypeError});}
    });
    run(prefix+'/subclass',()=>{class Sub extends C{}const e=new Sub('x',{button:65535,buttons:-1});record(prefix+'/subclass',{identity:e instanceof Sub&&e instanceof C,conversion:e.button===-1&&e.buttons===65535});let missing,call;try{new C();}catch(e){missing=e;}try{C('x');}catch(e){call=e;}record(prefix+'/required',{missing:missing instanceof realm.TypeError,call:call instanceof realm.TypeError});});
  }
  const executed=rows.reduce((n,r)=>n+Object.keys(r.checks).length,0),passed=rows.reduce((n,r)=>n+Object.values(r.checks).filter(Boolean).length,0),skipped=unexecuted.reduce((n,r)=>n+r.checks,0);
  return {total:executed+skipped,executed,passed,failed:executed-passed,unexecutedChecks:skipped,unexecuted,errors,rows,complete:errors.length===0&&skipped===0&&passed===executed};
};
