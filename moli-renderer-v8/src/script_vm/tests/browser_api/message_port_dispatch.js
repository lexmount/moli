function portDispatchProbe() {
 const rows=[];
 for(const state of ['fresh','started','closed','transferred'])for(const type of ['message','messageerror','custom',''])for(const borrowed of [false,true]){
  const channel=new MessageChannel(),port=channel.port1,trace=[];let clone;
  const listen=(name,capture)=>function(e){trace.push([name,this===port,e.target===port,e.currentTarget===port,e.eventPhase,e.isTrusted,e.composedPath().length,e.composedPath()[0]===port]);};
  port.addEventListener(type,listen('bubble',false));
  port.addEventListener(type,listen('capture',true),true);
  if(type==='message'||type==='messageerror')port['on'+type]=listen('handler',false);
  if(state==='started')port.start();
  if(state==='closed')port.close();
  if(state==='transferred')clone=structuredClone(port,{transfer:[port]});
  port.addEventListener(type,listen('late',false));
  const event=new Event(type,{cancelable:true});let returned=null,error=null;
  try{returned=(borrowed?EventTarget.prototype.dispatchEvent:port.dispatchEvent).call(port,event);}catch(e){error=e.name;}
  rows.push({state,type,borrowed,trace,returned,error,after:[event.target===port,event.currentTarget===null,event.eventPhase,event.composedPath().length]});
  port.close();channel.port2.close();clone?.close();
 }
 for(const action of ['once','remove','replace-handler','reactivate-handler','abort','capture-add','stop','stopImmediate','prevent','passive','return-false','recursive','close','transfer']){
  const channel=new MessageChannel(),port=channel.port1,controller=new AbortController(),trace=[];let nested=false,error=null,clone;
  const later=()=>trace.push('later'),handler=()=>{trace.push('handler');if(action==='return-false')return false;},replacement=()=>trace.push('replacement');
  port.addEventListener('message',function(e){trace.push('first');
   if(action==='close')port.close();
   if(action==='transfer'&&!clone)clone=structuredClone(port,{transfer:[port]});
   if(action==='remove')port.removeEventListener('message',later);
   if(action==='replace-handler')port.onmessage=replacement;
   if(action==='reactivate-handler'){port.onmessage=null;port.onmessage=replacement;}
   if(action==='abort')controller.abort();
   if(action==='capture-add')port.addEventListener('message',()=>trace.push('added'));
   if(action==='stop')e.stopPropagation();
   if(action==='stopImmediate')e.stopImmediatePropagation();
   if(action==='prevent'||action==='passive')e.preventDefault();
   if(action==='recursive'&&!nested){nested=true;try{port.dispatchEvent(e);}catch(err){trace.push(err.name);}}
  },{capture:action==='capture-add'||action==='stop',once:action==='once',passive:action==='passive'});
  port.onmessage=handler;port.addEventListener('message',later,{signal:controller.signal});
  const event=new Event('message',{cancelable:true});let returns=[];
  try{returns=[port.dispatchEvent(event),port.dispatchEvent(event)];}catch(e){error=e.name;}
  rows.push({action,trace,returns,error,after:[event.defaultPrevented,event.cancelBubble,event.eventPhase,event.currentTarget===null]});
  port.close();channel.port2.close();clone?.close();
 }
 return rows;
}
