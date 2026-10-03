globalThis.__inputEventProbe = realms => {
  const rows = [], errors = [], unexecuted = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const run = (label, work) => { try { work(); } catch (error) { errors.push({label, error: String(error.stack || error)}); } };
  const eq = (a,b) => JSON.stringify(a) === JSON.stringify(b);
  const units = s => Array.from({length:s.length},(_,i)=>s.charCodeAt(i));
  const keys = ['bubbles','cancelable','composed','detail','view','which','data','dataTransfer','inputType','isComposing','targetRanges'];
  for (const [r,w] of realms.entries()) {
    const C = w.InputEvent, p = `${r}/InputEvent`, other = realms[(r+1)%realms.length];
    const makeRange = (realm=w) => {
      const text=realm.document.createTextNode('abcdef');
      return new realm.StaticRange({startContainer:text,startOffset:1,endContainer:text,endOffset:4});
    };
    run(p+'/metadata',()=>{
      const event=new C('x'); record(p+'/constructor',{name:C.name==='InputEvent',length:C.length===1,parent:Object.getPrototypeOf(C.prototype)===w.UIEvent.prototype,ui:event instanceof w.UIEvent});
      for(const key of ['data','inputType','isComposing','dataTransfer']) {
        const d=Object.getOwnPropertyDescriptor(C.prototype,key);
        record(p+'/descriptor-'+key,{getter:typeof d?.get==='function',name:d?.get?.name==='get '+key,length:d?.get?.length===0,enumerable:d?.enumerable===true,configurable:d?.configurable===true,setter:d?.set===undefined,own:!Object.hasOwn(event,key)});
      }
      const d=Object.getOwnPropertyDescriptor(C.prototype,'getTargetRanges');
      record(p+'/method',{function:typeof d?.value==='function',name:d?.value?.name==='getTargetRanges',length:d?.value?.length===0,enumerable:d?.enumerable===true,configurable:d?.configurable===true,writable:d?.writable===true,own:!Object.hasOwn(event,'getTargetRanges')});
    });
    for(const [i,init] of [undefined,null,{},[],function(){},new Date(0)].entries()) run(p+'/defaults-'+i,()=>{
      const e=new C('x',init); record(p+'/defaults-'+i,{flags:!e.bubbles&&!e.cancelable&&!e.composed,ui:e.view===null&&e.detail===0&&e.which===0,data:e.data===null,inputType:e.inputType==='',composing:e.isComposing===false,dataTransfer:e.dataTransfer===null});
    });
    for(const [i,init] of [true,false,0,1,NaN,'', 'x',1n,Symbol('x')].entries()) run(p+'/dictionary-'+i,()=>{
      let error;try{new C('x',init);}catch(e){error=e;}record(p+'/dictionary-'+i,{typeError:error instanceof w.TypeError});
    });
    run(p+'/order',()=>{
      const trace=[],init=new Proxy({}, {get(o,k){trace.push(String(k));return undefined;}});
      new C({toString(){trace.push('type');return 'x';}},init);
      record(p+'/order',{order:eq(trace,['type',...keys])},trace);
    });
    for(const [i,key] of keys.entries()) run(p+'/getter-throw-'+key,()=>{
      const trace=[],sentinel={},init=new Proxy({}, {get(o,k){trace.push(String(k));if(k===key)throw sentinel;return undefined;}});
      let error;try{new C('x',init);}catch(e){error=e;}
      record(p+'/getter-throw-'+key,{same:error===sentinel,stops:eq(trace,keys.slice(0,i+1))},trace);
    });
    for(const key of ['data','inputType','detail','which']) run(p+'/conversion-'+key,()=>{
      const trace=[],sentinel={},init=new Proxy({}, {get(o,k){trace.push(String(k));if(k===key)return {[Symbol.toPrimitive](hint){trace.push(hint);throw sentinel;}};return undefined;}});
      let error;try{new C('x',init);}catch(e){error=e;}
      record(p+'/conversion-'+key,{same:error===sentinel,stops:eq(trace,[...keys.slice(0,keys.indexOf(key)+1),['data','inputType'].includes(key)?'string':'number'])},trace);
    });
    for(const [i,value] of [undefined,null,'','abc','\uD800','\uDC00','a\uD800b','\uD83D\uDE80',1n,true,123,['a','b']].entries()) run(p+'/data-'+i,()=>{
      const actual=new C('x',{data:value}).data;
      record(p+'/data-'+i,{value:actual===(value==null?null:String(value)),string:actual===null||typeof actual==='string'},typeof actual==='string'?units(actual):typeof actual);
    });
    for(const key of ['data','inputType']) run(p+'/'+key+'-symbol',()=>{
      let error;try{new C('x',{[key]:Symbol('x')});}catch(e){error=e;}record(p+'/'+key+'-symbol',{typeError:error instanceof w.TypeError});
    });
    run(p+'/data-hint',()=>{
      const trace=[],value={[Symbol.toPrimitive](hint){trace.push(hint);return '\uD800';}};
      const e=new C('x',{data:value}); record(p+'/data-hint',{hint:eq(trace,['string']),value:e.data==='\uD800'},units(e.data));
    });
    for(const inputType of ['', 'insertText','deleteContentBackward','insertFromPaste','historyUndo','formatBold']) run(p+'/inputType-'+inputType,()=>{
      const trace=[],e=new C('x',{inputType:{toString(){trace.push('string');return inputType;}}});record(p+'/inputType-'+inputType,{value:e.inputType===inputType,once:eq(trace,['string'])},e.inputType);
    });
    run(p+'/unknown-inputType',()=>{ const e=new C('x',{inputType:'custom\uD800'});record(p+'/unknown-inputType',{domString:e.inputType==='custom\uD800'},units(e.inputType)); });
    for(const [i,value] of [false,0,'',null,undefined,NaN,true,1,'x',{},1n].entries()) run(p+'/boolean-'+i,()=>{
      record(p+'/boolean-'+i,{boolean:new C('x',{isComposing:value}).isComposing===Boolean(value)});
    });
    run(p+'/ui',()=>{
      const e=new C('x',{bubbles:true,cancelable:true,composed:true,view:other,detail:2**32+5,which:-1,data:'a'});
      record(p+'/ui',{flags:e.bubbles&&e.cancelable&&e.composed,view:e.view===other,detail:e.detail===5,which:e.which===2**32-1});
    });
    run(p+'/dataTransfer',()=>{
      for(const [i,transfer] of [undefined,null,new w.DataTransfer(),new other.DataTransfer()].entries()) {
        const e=new C('x',{dataTransfer:transfer});record(p+'/dataTransfer-'+i,{identity:e.dataTransfer===(transfer??null)});
      }
      const valid=new w.DataTransfer(); Object.setPrototypeOf(valid,null);
      record(p+'/dataTransfer-reparent',{native:new C('x',{dataTransfer:valid}).dataTransfer===valid});
      const real=new w.DataTransfer(),revoked=Proxy.revocable(real,{});revoked.revoke();
      for(const [i,transfer] of [{},Object.create(w.DataTransfer.prototype),Object.create(real),new Proxy(real,{}),revoked.proxy,1,'x'].entries()) {
        let error;try{new C('x',{dataTransfer:transfer});}catch(e){error=e;}record(p+'/bad-transfer-'+i,{typeError:error instanceof w.TypeError});
      }
    });
    run(p+'/range-brand',()=>{
      const real=makeRange(),revoked=Proxy.revocable(real,{});revoked.revoke();
      for(const [i,range] of [{},Object.create(w.StaticRange.prototype),Object.create(real),new Proxy(real,{}),revoked.proxy,new w.Range(),null,1].entries()) {
        let error;try{new C('x',{targetRanges:[range]});}catch(e){error=e;}record(p+'/bad-range-'+i,{typeError:error instanceof w.TypeError});
      }
      for(const [i,sequence] of [null,1,'',{}, {length:1,0:real}, {[Symbol.iterator]:1}].entries()) {
        let error;try{new C('x',{targetRanges:sequence});}catch(e){error=e;}record(p+'/bad-sequence-'+i,{typeError:error instanceof w.TypeError});
      }
      const trace=[],sentinel={},iterable={[Symbol.iterator](){trace.push('iterator');return {next(){trace.push('next');return {done:false,value:{}};},return(){trace.push('return');return {};}};}};
      let error;try{new C('x',{targetRanges:iterable});}catch(e){error=e;}
      record(p+'/iterator-close',{typeError:error instanceof w.TypeError,noClose:eq(trace,['iterator','next'])},trace);
      let thrown;try{new C('x',{targetRanges:{[Symbol.iterator](){throw sentinel;}}});}catch(e){thrown=e;}record(p+'/iterator-throw',{same:thrown===sentinel});
    });
    run(p+'/range-results',()=>{
      if(typeof C.prototype.getTargetRanges!=='function'){unexecuted.push({label:p+'/range-results',checks:14,reason:'missing method'});return;}
      const ranges=[makeRange(),makeRange(other)],e=new C('x',{targetRanges:new Set(ranges)});
      const a=e.getTargetRanges(),b=e.getTargetRanges();
      record(p+'/range-copies',{length:a.length===2,array:a instanceof w.Array,fresh:a!==b,static:a.every(x=>x instanceof w.StaticRange),snapshot:a[0]!==b[0]&&a[0]!==ranges[0],start:a[0].startContainer===ranges[0].startContainer,offsets:a[0].startOffset===1&&a[0].endOffset===4});
      a.pop();record(p+'/array-mutation',{stored:e.getTargetRanges().length===2});
      const initial=b[0];ranges[0].startContainer.replaceData(2,1,'xyz');
      const now=e.getTargetRanges()[0];record(p+'/range-live',{old:initial.endOffset===4,updated:now.endOffset===6,original:ranges[0].endOffset===4});
      const empty=new C('x');record(p+'/range-empty',{empty:empty.getTargetRanges().length===0,fresh:empty.getTargetRanges()!==empty.getTargetRanges()});
      const input=[makeRange()],copy=new C('x',{targetRanges:input}); input.length=0; record(p+'/sequence-snapshot',{copy:copy.getTargetRanges().length===1});
    });
    run(p+'/range-native',()=>{
      if(typeof C.prototype.getTargetRanges!=='function'){unexecuted.push({label:p+'/range-native',checks:4,reason:'missing method'});return;}
      const range=makeRange(),text=range.startContainer,sentinel={};
      for(const obj of [range,text]) for(const key of obj===range?['startContainer','startOffset','endContainer','endOffset']:['nodeType','length','childNodes','ownerDocument']) Object.defineProperty(obj,key,{configurable:true,get(){throw sentinel;}});
      const event=new C('x',{targetRanges:[range]}),snapshot=event.getTargetRanges()[0];
      record(p+'/range-native',{start:snapshot.startContainer===text,end:snapshot.endContainer===text,startOffset:snapshot.startOffset===1,endOffset:snapshot.endOffset===4});
    });
    for(const field of ['data','inputType','isComposing','dataTransfer','getTargetRanges']) run(p+'/receiver-'+field,()=>{
      const d=Object.getOwnPropertyDescriptor(C.prototype,field),call=field==='getTargetRanges'?d?.value:d?.get;
      if(typeof call!=='function'){unexecuted.push({label:p+'/receiver-'+field,checks:7,reason:'missing binding'});return;}
      const e=new C('x'),foreign=new other.InputEvent('x');
      record(p+'/genuine-'+field,{foreign:field==='getTargetRanges'?call.call(foreign).length===0:Object.is(call.call(foreign),foreign[field]),own:field==='getTargetRanges'?call.call(e).length===0:Object.is(call.call(e),e[field])});
      const revoked=Proxy.revocable(e,{});revoked.revoke();
      for(const [i,receiver] of [{},Object.create(C.prototype),Object.create(e),new Proxy(e,{}),revoked.proxy].entries()) {
        let error;try{call.call(receiver);}catch(e){error=e;}record(p+'/receiver-'+field+'-'+i,{typeError:error instanceof w.TypeError});
      }
    });
    run(p+'/subclass',()=>{
      class Derived extends C {} const e=new Derived('beforeinput',{data:'abc',inputType:'insertText',dataTransfer:new w.DataTransfer()});
      record(p+'/subclass',{derived:e instanceof Derived,input:e instanceof C,data:e.data==='abc'});
      w.document.body.dispatchEvent(e);record(p+'/dispatch',{target:e.target===w.document.body,type:e.type==='beforeinput',phase:e.eventPhase===0,untrusted:!e.isTrusted});
    });
  }
  const executed=rows.reduce((n,r)=>n+Object.keys(r.checks).length,0),passed=rows.reduce((n,r)=>n+Object.values(r.checks).filter(Boolean).length,0),skipped=unexecuted.reduce((n,r)=>n+r.checks,0);
  return {total:executed+skipped,executed,passed,failed:executed-passed,unexecutedChecks:skipped,unexecuted,errors,rows,complete:errors.length===0&&skipped===0&&passed===executed};
};
