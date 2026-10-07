(() => {
  const checks=[],ns='http://www.w3.org/2000/svg';
  const check=(name,fn)=>{try {checks.push({name,passed:fn()===true});}catch(error){checks.push({name,passed:false,error:String(error)});}};
  const realms=[window,document.querySelector('iframe').contentWindow];
  const typeError=(realm,fn)=>{try {fn();}catch(error){return Object.getPrototypeOf(error)===realm.TypeError.prototype;}return false;};
  for(const [ownerIndex,owner] of realms.entries()) {
    const docs=[owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for(const [docIndex,doc] of docs.entries())for(const [calleeIndex,callee] of realms.entries())for(const tag of ['text','tspan','textPath']) {
      const element=doc.createElementNS(ns,tag),fn=callee.SVGTextContentElement.prototype.getCharNumAtPosition,prefix=`text-point/${ownerIndex}/${docIndex}/${calleeIndex}/${tag}`;
      check(prefix+'/arity',()=>fn.length===0);
      check(prefix+'/omitted',()=>fn.call(element)===-1);
      for(const [index,value] of [undefined,null,{},[],()=>{}, {x:1,y:2}, {x:NaN,y:Infinity,z:-Infinity,w:0}].entries())check(prefix+'/dictionary/'+index,()=>fn.call(element,value)===-1);
      for(const [index,value] of [0,true,'text',Symbol(),1n].entries())check(prefix+'/primitive/'+index,()=>typeError(callee,()=>fn.call(element,value)));
      check(prefix+'/member-order',()=>{const order=[],point={};for(const name of ['w','x','y','z'])Object.defineProperty(point,name,{get(){order.push(name);return {valueOf(){order.push(name+'-number');return 1;}};}});return fn.call(element,point)===-1 && order.join(',')==='w,w-number,x,x-number,y,y-number,z,z-number';});
      for(const name of ['w','x','y','z'])check(prefix+'/exception/'+name,()=>{const sentinel={},order=[],point={};for(const key of ['w','x','y','z'])Object.defineProperty(point,key,{get(){order.push(key);if(key===name)throw sentinel;return 0;}});try{fn.call(element,point);}catch(error){return error===sentinel && order.join(',')===['w','x','y','z'].slice(0,['w','x','y','z'].indexOf(name)+1).join(',');}return false;});
      check(prefix+'/ignored-extra',()=>{let n=0;return fn.call(element,{}, {get x(){n++;throw 1;}})===-1 && n===0;});
      let traps=0;const handler={get(){traps++;throw 1;},getPrototypeOf(){traps++;throw 2;}},author=new owner.Proxy(element,handler),revoked=owner.Proxy.revocable(element,handler);revoked.revoke();
      for(const [index,receiver] of [null,{},Object.create(element),Object.create(callee.SVGTextContentElement.prototype),author,revoked.proxy,doc,doc.createElementNS(ns,'g')].entries())check(prefix+'/brand/'+index,()=>{let n=0;return typeError(callee,()=>fn.call(receiver,{get x(){n++;throw 1;}})) && n===0 && traps===0;});
    }
  }
  globalThis.__svgTextPointResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  globalThis.__uiEventResults=globalThis.__svgTextPointResults;
  return true;
})()
