(() => {
  const child=document.querySelector('iframe').contentWindow, ns='http://www.w3.org/2000/svg';
  const checks=[];
  const throws=(action,Constructor,name)=>{try{action();return false;}catch(error){return error instanceof Constructor && (name===undefined || error.name===name);}};
  for (const realm of [window,child]) {
    for (const [kind,doc] of [['live',realm.document],['html',realm.document.implementation.createHTMLDocument('')],
      ['xml',new realm.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')]]) {
      for (const callee of [window,child]) {
        const label=(realm===window?'main':'child')+'/'+kind+'/'+(callee===window?'main':'child');
        const setup=()=>{
          const owner=doc.createElementNS(ns,'g');owner.setAttribute('transform','translate(2 3)');
          return {owner,base:owner.transform.baseVal.getItem(0),anim:owner.transform.animVal.getItem(0)};
        };
        const set=callee.SVGTransform.prototype.setMatrix;
        for (const [name,run] of Object.entries({
          'writable-aliases':()=>{const {base}=setup(),matrix=base.matrix;set.call(base,{a:7,m11:7,d:2});return base.matrix===matrix && matrix.a===7 && matrix.d===2 && base.type===1;},
          'writable-conflict':()=>{const {base,owner}=setup(),raw=owner.getAttribute('transform');return throws(()=>set.call(base,{a:7,m11:8}),callee.TypeError) && base.type===2 && base.matrix.e===2 && owner.getAttribute('transform')===raw;},
          'readonly-before-fixup':()=>{const {anim,owner}=setup(),raw=owner.getAttribute('transform');return throws(()=>set.call(anim,{a:7,m11:8}),callee.DOMException,'NoModificationAllowedError') && anim.type===2 && anim.matrix.e===2 && owner.getAttribute('transform')===raw;},
          'readonly-after-dictionary':()=>{const {anim}=setup();return throws(()=>set.call(anim,1),callee.TypeError);},
          'readonly-getter-error':()=>{const {anim}=setup(),marker={};try{set.call(anim,{get a(){throw marker}});return false;}catch(error){return error===marker;}},
          'readonly-complete-conversion':()=>{const {anim}=setup(),seen=[];const input={get a(){seen.push('a');return 7},get m11(){seen.push('m11');return 8}};return throws(()=>set.call(anim,input),callee.DOMException,'NoModificationAllowedError') && seen.join(',')==='a,m11';},
          'readonly-value-error':()=>{const {anim}=setup();return throws(()=>set.call(anim,{a:Symbol()}),callee.TypeError);},
          'receiver-before-conversion':()=>{const {base}=setup();let conversions=0;const input={get a(){conversions++;return 1}};return throws(()=>set.call(new Proxy(base,{}),input),callee.TypeError) && conversions===0;},
          'optional-identity':()=>{const {base}=setup(),matrix=base.matrix;set.call(base);return base.matrix===matrix && base.type===1 && matrix.a===1 && matrix.d===1 && matrix.e===0 && matrix.f===0;},
        })) {
          try { checks.push({name:label+'/'+name,passed:run()===true}); }
          catch(error) { checks.push({name:label+'/'+name,passed:false,error:String(error)}); }
        }
      }
    }
  }
  globalThis.__uiEventResults={complete:true,checks,total:checks.length,passed:checks.filter(r=>r.passed).length};
  return checks.every(r=>r.passed);
})();
