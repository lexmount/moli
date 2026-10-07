(() => {
  const checks=[],ns='http://www.w3.org/2000/svg';
  const check=(name,fn)=>{try{checks.push({name,passed:fn()===true});}catch(error){checks.push({name,passed:false,error:String(error)});}};
  const realms=[window,document.querySelector('iframe').contentWindow],names=['w','x','y','z'];
  const typeError=(realm,fn)=>{try{fn();}catch(error){return Object.getPrototypeOf(error)===realm.TypeError.prototype;}return false;};
  for(const [ownerIndex,owner] of realms.entries()) {
    const docs=[owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for(const [docIndex,doc] of docs.entries())for(const [calleeIndex,callee] of realms.entries()) {
      const line=doc.createElementNS(ns,'line'),matrix=new callee.DOMMatrix();
      const calls=[['DOMPoint.fromPoint',point=>callee.DOMPoint.fromPoint(point)],['DOMPointReadOnly.fromPoint',point=>callee.DOMPointReadOnly.fromPoint(point)],['DOMMatrix.transformPoint',point=>matrix.transformPoint(point)],['DOMQuad',point=>new callee.DOMQuad(point)],['isPointInFill',point=>callee.SVGGeometryElement.prototype.isPointInFill.call(line,point)],['isPointInStroke',point=>callee.SVGGeometryElement.prototype.isPointInStroke.call(line,point)]];
      for(const [name,call] of calls) {
        const prefix=`point-order/${ownerIndex}/${docIndex}/${calleeIndex}/${name}`;
        check(prefix+'/getter-number-order',()=>{const order=[],point={};for(const key of names)Object.defineProperty(point,key,{get(){order.push(key);return {valueOf(){order.push(key+'-number');return 1;}};}});call(point);return order.join(',')==='w,w-number,x,x-number,y,y-number,z,z-number';});
        check(prefix+'/inherited-dictionary',()=>{const order=[],prototype={};for(const key of names)Object.defineProperty(prototype,key,{get(){order.push(key);return 1;}});call(Object.create(prototype));return order.join(',')==='w,x,y,z';});
        check(prefix+'/proxy-dictionary',()=>{const order=[];call(new owner.Proxy({},{get(target,key){order.push(key);return 1;}}));return order.join(',')==='w,x,y,z';});
        for(const key of names)for(const number of [false,true])check(prefix+'/exception/'+key+'/'+number,()=>{const sentinel={},order=[],point={};for(const member of names)Object.defineProperty(point,member,{get(){order.push(member);if(member===key){if(!number)throw sentinel;return {valueOf(){order.push(member+'-number');throw sentinel;}};}return 1;}});try{call(point);}catch(error){const expected=names.slice(0,names.indexOf(key)+1);if(number)expected.push(key+'-number');return error===sentinel && order.join(',')===expected.join(',');}return false;});
        for(const [index,value] of [undefined,null,{},[],()=>{}].entries())check(prefix+'/empty-dictionary/'+index,()=>{call(value);return true;});
        for(const [index,value] of [0,true,'point',Symbol(),1n].entries())check(prefix+'/invalid-dictionary/'+index,()=>typeError(callee,()=>call(value)));
      }
    }
  }
  globalThis.__geometryPointOrderResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  globalThis.__uiEventResults=globalThis.__geometryPointOrderResults;
  return true;
})()
