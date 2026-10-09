(() => {
 const checks=[],diagnostics=[];
 const check=(name,action)=>{try{checks.push({name,passed:action()===true})}catch(error){checks.push({name,passed:false,error:String(error)})}};
 const throws=(C,action,name)=>{let error;try{action()}catch(caught){error=caught}return error instanceof C&&(name===undefined||error.name===name)};
 const frame=document.createElement('iframe');document.body.appendChild(frame);const popup=window.open('about:blank');
 try {
  for(const [label,w] of [['main',window],['iframe',frame.contentWindow],['popup',popup]]) {
   const P=w.MediaSource.prototype,L=w.SourceBufferList.prototype,source=new w.MediaSource();
   const getters=['sourceBuffers','activeSourceBuffers','readyState','duration','onsourceopen','onsourceended','onsourceclose'];
   const methods=[['addSourceBuffer',1],['removeSourceBuffer',1],['endOfStream',0],['setLiveSeekableRange',2],['clearLiveSeekableRange',0]];
   for(const [name,C] of [['MediaSource',w.MediaSource],['SourceBufferList',w.SourceBufferList]]){
    check(label+'/'+name+'/interface',()=>typeof C==='function'&&Object.getPrototypeOf(C.prototype)===w.EventTarget.prototype);
    check(label+'/'+name+'/function call',()=>throws(w.TypeError,()=>C()));
   }
   check(label+'/SourceBufferList/illegal constructor',()=>throws(w.TypeError,()=>new w.SourceBufferList()));
   for(const name of getters)check(label+'/MediaSource/descriptor/'+name,()=>{
    const d=Object.getOwnPropertyDescriptor(P,name),set=name==='duration'||name.startsWith('on');
    return d.enumerable&&d.configurable&&d.get.length===0&&d.get.name==='get '+name&&(set?d.set.length===1&&d.set.name==='set '+name:d.set===undefined);
   });
   for(const [name,length] of methods)check(label+'/MediaSource/method/'+name,()=>{const d=Object.getOwnPropertyDescriptor(P,name);return d.enumerable&&d.configurable&&d.writable&&d.value.length===length&&d.value.name===name});
   for(const name of ['length','onaddsourcebuffer','onremovesourcebuffer'])check(label+'/SourceBufferList/descriptor/'+name,()=>{const d=Object.getOwnPropertyDescriptor(L,name);return d.enumerable&&d.configurable&&d.get.length===0&&(name==='length'?d.set===undefined:d.set.length===1)});
   check(label+'/SourceBufferList/iterator descriptor',()=>{const d=Object.getOwnPropertyDescriptor(L,Symbol.iterator);return d.value===w.Array.prototype.values&&!d.enumerable&&d.configurable&&d.writable});
   check(label+'/MediaSource/closed defaults',()=>source.readyState==='closed'&&Number.isNaN(source.duration)&&source.onsourceopen===null&&source.onsourceended===null&&source.onsourceclose===null);
   for(const name of ['sourceBuffers','activeSourceBuffers'])check(label+'/MediaSource/native list/'+name,()=>{const list=source[name];return list instanceof w.SourceBufferList&&list instanceof w.EventTarget&&!w.Array.isArray(list)&&list===source[name]&&list.length===0&&list[0]===undefined&&Object.keys(list).length===0&&Object.prototype.toString.call(list)==='[object SourceBufferList]'&&[...list].length===0});
   check(label+'/MediaSource/distinct lists',()=>source.sourceBuffers!==source.activeSourceBuffers&&source.sourceBuffers!==new w.MediaSource().sourceBuffers);
   for(const [name,args,C,errorName] of [
    ['addSourceBuffer',[],w.TypeError],['addSourceBuffer',[''],w.TypeError],['addSourceBuffer',['video/moli'],w.DOMException,'NotSupportedError'],['addSourceBuffer',['video/mp4;codecs="avc1.4d001e"'],w.DOMException,'InvalidStateError'],
    ['removeSourceBuffer',[],w.TypeError],['removeSourceBuffer',[{}],w.TypeError],['endOfStream',[],w.DOMException,'InvalidStateError'],['endOfStream',['decode'],w.DOMException,'InvalidStateError'],['endOfStream',['network'],w.DOMException,'InvalidStateError'],['endOfStream',[undefined],w.DOMException,'InvalidStateError'],['endOfStream',[''],w.TypeError],['endOfStream',['other'],w.TypeError],
    ['setLiveSeekableRange',[],w.TypeError],['setLiveSeekableRange',[0],w.TypeError],['setLiveSeekableRange',[NaN,1],w.TypeError],['setLiveSeekableRange',[0,Infinity],w.TypeError],['setLiveSeekableRange',[-1,0],w.DOMException,'InvalidStateError'],['setLiveSeekableRange',[2,1],w.DOMException,'InvalidStateError'],['setLiveSeekableRange',[0,1],w.DOMException,'InvalidStateError'],['clearLiveSeekableRange',[],w.DOMException,'InvalidStateError']
   ])check(label+'/MediaSource/error/'+name+'/'+args.map(String).join(','),()=>throws(C,()=>Reflect.apply(P[name],source,args),errorName));
   const setter=Object.getOwnPropertyDescriptor(P,'duration')?.set;
   for(const [index,value] of [undefined,NaN,-1,-Infinity,0,-0,1,Infinity].entries())check(label+'/duration/value/'+index,()=>throws(value===undefined||Number.isNaN(value)||value<0?w.TypeError:w.DOMException,()=>setter.call(source,value),value!==undefined&&!Number.isNaN(value)&&value>=0?'InvalidStateError':undefined));
   for(const kind of ['addSourceBuffer','endOfStream','duration','setLiveSeekableRange'])check(label+'/conversion/'+kind,()=>{
    const order=[],marker={};const first={toString(){order.push('first');throw marker},valueOf(){order.push('first');throw marker}},second={valueOf(){order.push('second');return 1}};
    let error;try{if(kind==='duration')setter.call(source,first);else P[kind].call(source,first,second)}catch(caught){error=caught}
    return error===marker&&order.join()==='first';
   });
   const bad=[{},Object.create(P),Object.create(source),new Proxy(source,{})];const revoked=Proxy.revocable(source,{});revoked.revoke();bad.push(revoked.proxy);
   for(const [index,receiver] of bad.entries()) {
    for(const name of getters)check(label+'/invalid getter/'+index+'/'+name,()=>throws(w.TypeError,()=>Object.getOwnPropertyDescriptor(P,name).get.call(receiver)));
    for(const name of ['duration','onsourceopen','onsourceended','onsourceclose'])check(label+'/invalid setter/'+index+'/'+name,()=>{let conversions=0;return throws(w.TypeError,()=>Object.getOwnPropertyDescriptor(P,name).set.call(receiver,{valueOf(){conversions++;throw Error()}}))&&conversions===0});
    for(const [name] of methods)check(label+'/invalid method/'+index+'/'+name,()=>{let conversions=0;const argument={toString(){conversions++;throw Error()},valueOf(){conversions++;throw Error()}};return throws(w.TypeError,()=>P[name].call(receiver,argument,argument))&&conversions===0});
   }
   for(const name of ['onsourceopen','onsourceended','onsourceclose'])check(label+'/handler order/'+name,()=>{
    const events=[],event=name.slice(2),handler=function(e){events.push(this===source&&e.target===source?'handler':'bad');return false};
    source.addEventListener(event,()=>events.push('first'));source[name]=()=>events.push('old');source.addEventListener(event,()=>events.push('last'));source[name]=handler;
    const canceled=source.dispatchEvent(new w.Event(event,{cancelable:true}));source[name]=null;
    return canceled===false&&events.join()==='first,handler,last'&&source[name]===null;
   });
   for(const name of ['onaddsourcebuffer','onremovesourcebuffer'])check(label+'/list handler/'+name,()=>{
    const list=source.sourceBuffers,events=[],event=name.slice(2);list.addEventListener(event,()=>events.push('first'));list[name]=()=>events.push('old');list.addEventListener(event,()=>events.push('last'));list[name]=function(e){events.push(this===list&&e.target===list?'handler':'bad');return false};
    const result=list.dispatchEvent(new w.Event(event,{cancelable:true}));list[name]=null;return !result&&events.join()==='first,handler,last'&&list[name]===null;
   });
   for(const name of ['length','onaddsourcebuffer','onremovesourcebuffer'])for(const [index,receiver] of [{},Object.create(source.sourceBuffers ?? null),new Proxy(source.sourceBuffers ?? {},{})].entries())check(label+'/list invalid receiver/'+name+'/'+index,()=>throws(w.TypeError,()=>Object.getOwnPropertyDescriptor(L,name).get.call(receiver)));
   for(const index of ['0','4294967294'])check(label+'/list readonly index/'+index,()=>{const list=source.sourceBuffers;return Reflect.set(list,index,{})===false&&!Object.hasOwn(list,index)&&Reflect.defineProperty(list,index,{value:{},configurable:true})===false&&Reflect.deleteProperty(list,index)===true});
   for(const key of ['4294967295','01','-1'])check(label+'/list expando/'+key,()=>{const list=source.sourceBuffers;return Reflect.set(list,key,'value')&&list[key]==='value'&&Reflect.deleteProperty(list,key)});
   check(label+'/list prevent extensions',()=>Reflect.preventExtensions(source.sourceBuffers)===false&&Object.isExtensible(source.sourceBuffers));
   check(label+'/list intrinsic realm',()=>{const other=label==='main'?frame.contentWindow:window;return Object.getOwnPropertyDescriptor(other.MediaSource.prototype,'sourceBuffers').get.call(source) instanceof w.SourceBufferList});
   check(label+'/list intrinsic survives author global',()=>{const intrinsic=w.SourceBufferList;try{Object.defineProperty(w,'SourceBufferList',{get(){throw Error('author constructor getter')},configurable:true});const list=new w.MediaSource().sourceBuffers;return list instanceof intrinsic}finally{Object.defineProperty(w,'SourceBufferList',{value:intrinsic,writable:true,configurable:true})}});
   diagnostics.push({realm:label,sourceTag:Object.prototype.toString.call(source),state:source.readyState,listLength:source.sourceBuffers?.length});
  }
 } finally {frame.remove();popup?.close();}
 globalThis.__uiEventResults={complete:true,checks,diagnostics,passed:checks.filter(row=>row.passed).length,total:checks.length};return checks.every(row=>row.passed);
})()
