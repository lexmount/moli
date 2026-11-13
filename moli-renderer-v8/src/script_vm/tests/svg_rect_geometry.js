(() => {
  'use strict';
  const checks=[];
  function check(name,fn){try{checks.push({name,passed:!!fn()});}catch(error){checks.push({name,passed:false,error:String(error)});}}
  function errorOf(fn){try{fn();}catch(error){return error;}return null;}
  const frame=document.createElement('iframe');document.body.appendChild(frame);
  const realms=[['main',globalThis],['child',frame.contentWindow]];
  const fields=['x','y','width','height'];
  const ns='http://www.w3.org/2000/svg';
  try {
    for(const [label,w] of realms){
      check(`${label}/distinct interface`,()=>w.SVGRect!==w.DOMRect);
      check(`${label}/illegal constructor`,()=>errorOf(()=>new w.SVGRect()) instanceof w.TypeError);
      for(const windowless of [false,true]){
        const doc=windowless?w.document.implementation.createHTMLDocument(''):w.document;
        const root=doc.createElementNS(ns,'svg');doc.body.appendChild(root);
        const prefix=`${label}/${windowless?'windowless':'live'}`;
        const rect=root.createSVGRect(),animated=root.viewBox,base=animated.baseVal,anim=animated.animVal;
        for(const [kind,value] of [['factory',rect],['base',base],['anim',anim]]){
          check(`${prefix}/${kind}/native identity`,()=>Object.getPrototypeOf(value)===w.SVGRect.prototype&&value instanceof w.SVGRect&&!(value instanceof w.DOMRectReadOnly));
          check(`${prefix}/${kind}/tag`,()=>w.Object.prototype.toString.call(value)==='[object SVGRect]');
          check(`${prefix}/${kind}/private storage`,()=>!fields.some(name=>Object.hasOwn(value,name)));
          check(`${prefix}/${kind}/not serializable`,()=>errorOf(()=>structuredClone(value))?.name==='DataCloneError');
        }
        check(`${prefix}/same objects`,()=>root.viewBox===animated&&animated.baseVal===base&&animated.animVal===anim&&base!==anim);
        check(`${prefix}/readonly baseVal`,()=>!Reflect.set(animated,'baseVal',null)&&animated.baseVal===base);
        check(`${prefix}/readonly animVal`,()=>!Reflect.set(animated,'animVal',null)&&animated.animVal===anim);
        for(const [callerLabel,caller] of realms){
          for(const name of fields){
            const {get,set}=Object.getOwnPropertyDescriptor(caller.SVGRect.prototype,name);
            rect[name]=0;
            check(`${prefix}/${callerLabel}/${name}/cross realm read`,()=>get.call(rect)===0);
            for(const [kind,value] of [['object',{}],['forged',Object.create(w.SVGRect.prototype)],['DOMRect',new w.DOMRect()],['proxy',new w.Proxy(rect,{get(){throw Error('trap');}})]]){
              check(`${prefix}/${callerLabel}/${name}/${kind}/getter rejects`,()=>errorOf(()=>get.call(value)) instanceof caller.TypeError);
              check(`${prefix}/${callerLabel}/${name}/${kind}/setter brands first`,()=>{let calls=0;const error=errorOf(()=>set.call(value,{valueOf(){calls++;return 3;}}));return error instanceof caller.TypeError&&calls===0;});
            }
            check(`${prefix}/${callerLabel}/${name}/float32`,()=>{set.call(rect,.1);return get.call(rect)===Math.fround(.1);});
            for(const [kind,value] of [['NaN',NaN],['Infinity',Infinity],['overflow',1e40],['undefined',undefined],['bad string','no'],['symbol',Symbol()],['BigInt',1n]]){
              check(`${prefix}/${callerLabel}/${name}/${kind}/rejects unchanged`,()=>{const before=get.call(rect);return errorOf(()=>set.call(rect,value)) instanceof caller.TypeError&&get.call(rect)===before;});
            }
            check(`${prefix}/${callerLabel}/${name}/null converts`,()=>{set.call(rect,null);return get.call(rect)===0;});
            check(`${prefix}/${callerLabel}/${name}/readonly conversion`,()=>{let calls=0;const error=errorOf(()=>set.call(anim,{valueOf(){calls++;return 7;}}));return calls===1&&error?.name==='NoModificationAllowedError'&&error.code===7;});
            check(`${prefix}/${callerLabel}/${name}/conversion error precedes readonly`,()=>{const sentinel={};return errorOf(()=>set.call(anim,{valueOf(){throw sentinel;}}))===sentinel;});
            check(`${prefix}/${callerLabel}/${name}/DOMRect receiver rejected`,()=>errorOf(()=>Object.getOwnPropertyDescriptor(caller.DOMRect.prototype,name).get.call(rect)) instanceof caller.TypeError);
          }
        }
        root.setAttribute('viewBox','10 20 30 40');
        check(`${prefix}/retained synchronization`,()=>[base.x,base.y,base.width,base.height].join()==='10,20,30,40'&&[anim.x,anim.y,anim.width,anim.height].join()==='10,20,30,40');
        const x=Object.getOwnPropertyDescriptor(w.SVGRect.prototype,'x');
        check(`${prefix}/reentrant mutation`,()=>{let calls=0;x.set.call(base,{valueOf(){calls++;root.setAttribute('viewBox','50 60 70 80');return 7;}});return calls===1&&[base.x,base.y,base.width,base.height].join()==='7,60,70,80'&&anim.x===7;});
        check(`${prefix}/factory detached from owner`,()=>{rect.x=9;return root.getAttribute('viewBox')==='7 60 70 80';});
        if(windowless)check(`${prefix}/no synthetic Window`,()=>doc.defaultView===null);
        root.remove();
      }
    }
  } finally {frame.remove();}
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  return globalThis.__uiEventResults;
})()
