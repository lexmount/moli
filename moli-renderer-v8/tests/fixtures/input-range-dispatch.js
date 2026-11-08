(() => {
  const assert=(value,label)=>{if(!value)throw Error(label);};
  const host=document.createElement('div');document.body.append(host);
  const shadowHost=document.createElement('div');host.append(shadowHost);
  const shadow=shadowHost.attachShadow({mode:'open'});
  const makeEvent=(text,type='range-lifecycle',init={})=>new InputEvent(type,{cancelable:true,targetRanges:[new StaticRange({startContainer:text,startOffset:0,endContainer:text,endOffset:2})],...init});
  const targets=[['element',host,true],['document',document,true],['detached',document.createElement('div'),true],['fragment',document.createDocumentFragment(),true],['shadow',shadow,true],['window',window,false],['plain',new EventTarget(),false]];
  for(const [name,target,clears] of targets){
    const text=document.createTextNode('abc');host.append(text);
    const event=makeEvent(text),snapshot=event.getTargetRanges()[0],seen=[];
    const first=e=>{seen.push(e.getTargetRanges()[0].endOffset);text.deleteData(0,1);};
    const second=e=>{seen.push(e.getTargetRanges()[0].endOffset);};
    target.addEventListener(event.type,first);target.addEventListener(event.type,second);
    try{assert(target.dispatchEvent(event)===true,name+' dispatch result');}finally{target.removeEventListener(event.type,first);target.removeEventListener(event.type,second);}
    assert(seen.join()==='2,1',name+' ranges stay live throughout listeners');
    assert(event.getTargetRanges().length===(clears?0:1),name+' dispatch lifetime boundary');
    assert(snapshot.endOffset===2,name+' retained snapshots stay static');text.remove();
  }
  const text=document.createTextNode('abc');host.append(text);
  const stopped=makeEvent(text);stopped.stopPropagation();let calls=0;
  const count=()=>calls++;host.addEventListener(stopped.type,count);
  host.dispatchEvent(stopped);host.removeEventListener(stopped.type,count);
  assert(calls===0 && stopped.getTargetRanges().length===0,'pre-stopped dispatch still clears');
  const canceled=makeEvent(text);canceled.preventDefault();
  assert(host.dispatchEvent(canceled)===false && canceled.getTargetRanges().length===0,'canceled dispatch clears');
  const reentrant=makeEvent(text);let nestedError;
  const nested=event=>{
    try{host.dispatchEvent(event);}catch(error){nestedError=error;}
    assert(nestedError?.name==='InvalidStateError' && event.getTargetRanges().length===1,'failed reentrant dispatch must not clear outer ranges');
  };
  host.addEventListener(reentrant.type,nested,{once:true});host.dispatchEvent(reentrant);
  assert(reentrant.getTargetRanges().length===0,'outer dispatch clears after reentrant failure');
  let afterThrow=false;const failed=makeEvent(text,'range-throw');
  const cancelError=event=>event.preventDefault();window.addEventListener('error',cancelError);
  host.addEventListener(failed.type,()=>{throw Error('intentional listener exception');},{once:true});
  host.addEventListener(failed.type,event=>{afterThrow=event.getTargetRanges().length===1;},{once:true});
  try{host.dispatchEvent(failed);}finally{window.removeEventListener('error',cancelError);}
  assert(afterThrow && failed.getTargetRanges().length===0,'throwing listener does not skip native cleanup');
  assert(failed.eventPhase===Event.NONE && failed.currentTarget===null,'ordinary dispatch fields still clear');
  host.dispatchEvent(failed);assert(failed.getTargetRanges().length===0,'redispatch never restores old ranges');
  const frame=document.createElement('iframe');host.append(frame);const child=frame.contentWindow;
  const childText=child.document.createTextNode('abc');child.document.body.append(childText);
  const childEvent=new child.InputEvent('range-frame',{targetRanges:[new child.StaticRange({startContainer:childText,startOffset:0,endContainer:childText,endOffset:2})]});
  child.document.body.addEventListener('range-frame',()=>frame.remove(),{once:true});
  Node.prototype.dispatchEvent.call(child.document.body,childEvent);
  assert(childEvent.getTargetRanges().length===0,'retired child owner does not retain dispatch ranges');
  host.remove();return true;
})()
