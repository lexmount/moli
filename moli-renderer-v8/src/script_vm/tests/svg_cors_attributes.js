(() => {
  const checks=[];
  const check=(name,fn)=>{try{checks.push({name,passed:fn()===true});}catch(error){checks.push({name,passed:false,error:String(error)});}};
  const ns='http://www.w3.org/2000/svg',realms=[window,document.querySelector('iframe').contentWindow];
  const keywords=[['','anonymous'],['anonymous','anonymous'],['ANONYMOUS','anonymous'],['use-credentials','use-credentials'],['USE-CREDENTIALS','use-credentials'],['invalid','anonymous'],[' use-credentials ','anonymous'],['\ud800','anonymous']];
  const typeError=(realm,fn)=>{try{fn();}catch(error){return Object.getPrototypeOf(error)===realm.TypeError.prototype;}return false;};
  for(const [ownerIndex,owner] of realms.entries()) {
    const docs=[owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for(const [docIndex,doc] of docs.entries())for(const [calleeIndex,callee] of realms.entries()) {
      for(const [tag,iface,animated] of [['image','SVGImageElement',false],['script','SVGScriptElement',false],['feImage','SVGFEImageElement',true]]) {
        const prefix=`${ownerIndex}/${docIndex}/${calleeIndex}/${tag}`,element=doc.createElementNS(ns,tag);(doc.body||doc.documentElement).appendChild(element);
        const descriptor=Object.getOwnPropertyDescriptor(callee[iface].prototype,'crossOrigin'),get=descriptor?.get,set=descriptor?.set;
        const value=element.crossOrigin;
        check(prefix+'/descriptor',()=>descriptor.enumerable&&descriptor.configurable&&get.length===0&&(animated?set===undefined:set.length===1)&&!Object.hasOwn(element,'crossOrigin'));
        check(prefix+'/missing',()=>animated?get.call(element)===value&&value.baseVal===''&&value.animVal===''&&Object.getPrototypeOf(value)===owner.SVGAnimatedString.prototype:get.call(element)===null);
        for(const [raw,expected] of keywords) {
          check(prefix+'/content/'+JSON.stringify(raw),()=>{element.setAttribute('crossorigin',raw);return animated?get.call(element)===value&&value.baseVal===expected&&value.animVal===expected:get.call(element)===expected;});
          check(prefix+'/set/'+JSON.stringify(raw),()=>{if(animated)value.baseVal=raw;else set.call(element,raw);return element.getAttribute('crossorigin')===raw&&!Object.hasOwn(element,'crossOrigin')&&(animated?value.baseVal===expected&&value.animVal===expected:get.call(element)===expected);});
        }
        check(prefix+'/namespace-isolation',()=> {element.removeAttribute('crossorigin');element.setAttributeNS('urn:other','crossorigin','use-credentials');return animated?value.baseVal===''&&value.animVal==='':get.call(element)===null;});
        element.removeAttributeNS('urn:other','crossorigin');
        if(!animated) {
          for(const [name,input] of [['null',null],['undefined',undefined]])check(prefix+'/nullable/'+name,()=>{element.setAttribute('crossorigin','use-credentials');set.call(element,input);return !element.hasAttribute('crossorigin')&&get.call(element)===null;});
          check(prefix+'/arity',()=>typeof set==='function'&&typeError(callee,()=>set.call(element)));
          check(prefix+'/conversion-once',()=>{let n=0;set.call(element,{toString(){n++;return 'USE-CREDENTIALS';}});return n===1&&element.getAttribute('crossorigin')==='USE-CREDENTIALS'&&get.call(element)==='use-credentials';});
          check(prefix+'/conversion-exception',()=>{const sentinel={};let n=0;try{set.call(element,{toString(){n++;throw sentinel;}});}catch(error){return error===sentinel&&n===1&&element.getAttribute('crossorigin')==='USE-CREDENTIALS';}return false;});
          check(prefix+'/symbol',()=>typeof set==='function'&&typeError(callee,()=>set.call(element,Symbol()))&&element.getAttribute('crossorigin')==='USE-CREDENTIALS');
          let traps=0;const proxy=new Proxy(element,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});const revoked=Proxy.revocable(element,{});revoked.revoke();
          for(const [index,receiver] of [{},Object.create(callee[iface].prototype),Object.create(element),proxy,revoked.proxy,doc,doc.createElement('img'),doc.createElementNS(ns,tag==='image'?'script':'image')].entries()) {
            check(prefix+'/getter-brand/'+index,()=>typeof get==='function'&&typeError(callee,()=>get.call(receiver))&&traps===0);
            check(prefix+'/setter-brand/'+index,()=>{let n=0;return typeof set==='function'&&typeError(callee,()=>set.call(receiver,{toString(){n++;return 'anonymous';}}))&&n===0&&traps===0;});
          }
        } else {
          check(prefix+'/retained-value-removal',()=>{element.removeAttribute('crossorigin');return get.call(element)===value&&value.baseVal===''&&value.animVal==='';});
          check(prefix+'/readonly',()=>typeError(window,()=>{(function(){'use strict';element.crossOrigin='anonymous';})();}));
          for(const [index,receiver] of [{},Object.create(callee[iface].prototype),Object.create(element),new Proxy(element,{}),doc.createElementNS(ns,'image')].entries())check(prefix+'/getter-brand/'+index,()=>typeof get==='function'&&typeError(callee,()=>get.call(receiver)));
        }
        check(prefix+'/clone',()=>{element.setAttribute('crossorigin','use-credentials');const clone=element.cloneNode();return animated?clone.crossOrigin!==value&&clone.crossOrigin.baseVal==='use-credentials'&&value.baseVal==='use-credentials':get.call(clone)==='use-credentials';});
        check(prefix+'/detached',()=>{element.remove();element.setAttribute('crossorigin','invalid');return animated?get.call(element)===value&&value.baseVal==='anonymous':get.call(element)==='anonymous';});
      }
    }
  }
  globalThis.__svgCorsResults={complete:true,total:checks.length,passed:checks.filter(x=>x.passed).length,checks};globalThis.__uiEventResults=globalThis.__svgCorsResults;return true;
})()
