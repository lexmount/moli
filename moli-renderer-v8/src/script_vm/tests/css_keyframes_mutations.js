((bindings, values, label) => {
  const checks=[];
  const check=(name,fn)=>{try{const result=fn();checks.push({name:label+': '+name,passed:result===true,detail:result===true?null:String(result)});}catch(error){checks.push({name:label+': '+name,passed:false,detail:String(error)});}};
  const setter=Object.getOwnPropertyDescriptor(bindings.CSSKeyframeRule.prototype,'keyText').set;
  const append=bindings.CSSKeyframesRule.prototype.appendRule;
  const find=bindings.CSSKeyframesRule.prototype.findRule;
  const remove=bindings.CSSKeyframesRule.prototype.deleteRule;
  const syntaxError=error=>error instanceof bindings.DOMException && error.name==='SyntaxError' && error.code===12 && !(error instanceof bindings.SyntaxError);
  for(const mode of ['attached','constructed','detached']) {
    let owner;
    const sheet=mode==='attached' ? (()=>{owner=values.document.createElement('style');owner.textContent='@keyframes original {from {left:0px} to {left:100px}}';values.document.head.appendChild(owner);return owner.sheet;})() : new values.CSSStyleSheet();
    if(mode!=='attached')sheet.replaceSync('@keyframes original {from {left:0px} to {left:100px}}');
    const frames=sheet.cssRules[0],list=frames.cssRules,frame=list[0],declaration=frame.style;
    if(mode==='detached')sheet.deleteRule(0);
    const snapshot=()=>JSON.stringify([frame.keyText,frame.cssText,declaration.cssText,frames.cssText,frames.length,sheet.cssRules.length]);
    const initial=snapshot();
    const unchanged=()=>snapshot()===initial && frames.cssRules===list && list[0]===frame && frame.style===declaration;
    try {
      for(const [index,value] of ['', ' ', 'body', '-1%', '101%', '0', '0px', 'from,', 'from to', '0%, broken', '50% {}', '@keyframes k{}', '50%;', 'calc(50px)', undefined, null, false, 0, 1n, {}].entries()) {
        check(mode+' invalid keyText '+index+' throws DOM SyntaxError',()=>{let error;try{setter.call(frame,value);}catch(caught){error=caught;}return syntaxError(error);});
        check(mode+' invalid keyText '+index+' preserves state and identities',unchanged);
      }
      check(mode+' omitted setter value converts undefined',()=>{let error;try{setter.call(frame);}catch(caught){error=caught;}return syntaxError(error) && unchanged();});
      check(mode+' symbol keyText throws TypeError',()=>{let error;try{setter.call(frame,Symbol('selector'));}catch(caught){error=caught;}return error instanceof bindings.TypeError && unchanged();});
      check(mode+' keyText conversion exception identity',()=>{const sentinel=new values.Error('keyText');let calls=0,error;try{setter.call(frame,{[Symbol.toPrimitive](hint){calls++;if(hint!=='string')throw Error(hint);throw sentinel;}});}catch(caught){error=caught;}return error===sentinel && calls===1 && unchanged();});
      for(const [index,value] of ['', ' ', 'body {}', '-1% {}', '101% {}', '50px {}', '50% trailing {}', 'from, {}', '@media all {}', '50%;', 'from {} to {}', '0%, bad {}', undefined, null, false, 0, 1n, {}].entries()) {
        check(mode+' invalid appendRule '+index+' returns undefined',()=>append.call(frames,value)===undefined);
        check(mode+' invalid appendRule '+index+' preserves state and identities',unchanged);
      }
      check(mode+' appendRule missing argument throws TypeError',()=>{let error;try{append.call(frames);}catch(caught){error=caught;}return error instanceof bindings.TypeError && unchanged();});
      check(mode+' appendRule symbol throws TypeError',()=>{let error;try{append.call(frames,Symbol('rule'));}catch(caught){error=caught;}return error instanceof bindings.TypeError && unchanged();});
      check(mode+' appendRule conversion exception identity',()=>{const sentinel=new values.Error('appendRule');let calls=0,error;try{append.call(frames,{toString(){calls++;throw sentinel;}});}catch(caught){error=caught;}return error===sentinel && calls===1 && unchanged();});
      check(mode+' receiver validation precedes mutation conversion',()=>{let calls=0,error;try{setter.call(frames,{toString(){calls++;return '50%';}});}catch(caught){error=caught;}return error instanceof bindings.TypeError && calls===0 && unchanged();});
      check(mode+' valid keyText converts once and normalizes',()=>{let calls=0;const result=setter.call(frame,{[Symbol.toPrimitive](hint){calls++;if(hint!=='string')throw Error(hint);return ' 25% , to ';}});return result===undefined && calls===1 && frame.keyText==='25%, 100%' && frame.style===declaration && list[0]===frame && find.call(frames,'25%, to')===frame;});
      check(mode+' valid appendRule converts once and appends',()=>{let calls=0;const result=append.call(frames,{toString(){calls++;return '75% {left:75px}';}});return result===undefined && calls===1 && frames.length===3 && list[2]===find.call(frames,'75%') && list[2].style.left==='75px';});
      const last=list[2];
      check(mode+' duplicate selector returns last rule',()=>{append.call(frames,'75% {left:76px}');return frames.length===4 && find.call(frames,'75%')===list[3] && list[2]===last;});
      check(mode+' deleteRule deletes last duplicate only',()=>{remove.call(frames,'75%');return frames.length===3 && find.call(frames,'75%')===last;});
      check(mode+' selector list ordering stays significant',()=>find.call(frames,'100%, 25%')===null && find.call(frames,'25%, 100%')===frame);
      check(mode+' invalid findRule and deleteRule do not mutate',()=>{const before=snapshot();const result=find.call(frames,'body');remove.call(frames,'body');return result===null && snapshot()===before && frames.cssRules===list;});
    } finally {owner?.remove();}
  }
  return checks;
})
