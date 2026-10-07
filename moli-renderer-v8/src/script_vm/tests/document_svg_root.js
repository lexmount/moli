(() => {
  const checks = [], ns = 'http://www.w3.org/2000/svg';
  const check = (name, fn) => {try {checks.push({name,passed:fn()===true});} catch(error) {checks.push({name,passed:false,error:String(error)});}};
  const realms=[window,document.querySelector('iframe').contentWindow];
  const typeError=(realm,fn)=>{try {fn();} catch(error) {return Object.getPrototypeOf(error)===realm.TypeError.prototype;}return false;};
  for (const [ownerIndex,owner] of realms.entries()) {
    const docs=[owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"><g><svg/></g></svg>','image/svg+xml'),owner.document.implementation.createDocument(null,''),new owner.DOMParser().parseFromString('<svg xmlns="urn:other"/>','application/xml')];
    for (const [docIndex,doc] of docs.entries()) for (const [calleeIndex,callee] of realms.entries()) {
      const prefix=`root/${ownerIndex}/${docIndex}/${calleeIndex}`,descriptor=Object.getOwnPropertyDescriptor(callee.Document.prototype,'rootElement'),get=descriptor?.get;
      check(prefix+'/descriptor',()=>!!descriptor && descriptor.enumerable && descriptor.configurable && typeof get==='function' && get.length===0 && descriptor.set===undefined);
      check(prefix+'/identity',()=>{const expected=docIndex===2 || docIndex===3 ? doc.documentElement : null;return get.call(doc)===expected && get.call(doc)===expected;});
      check(prefix+'/receiver-realm',()=>{const root=get.call(doc);return root===null || Object.getPrototypeOf(root)===owner.SVGSVGElement.prototype;});
      check(prefix+'/own-shadowing',()=>{const original=doc.documentElement;Object.defineProperty(doc,'documentElement',{value:{},configurable:true});try {return get.call(doc)===(docIndex===2 || docIndex===3 ? original : null);} finally {delete doc.documentElement;}});
      check(prefix+'/readonly',()=>typeError(window,()=>{(function(){'use strict';doc.rootElement={};})();}));
      let traps=0;const handler={get(){traps++;throw 1;},getPrototypeOf(){traps++;throw 2;}};
      const author=new owner.Proxy(doc,handler),revoked=owner.Proxy.revocable(doc,handler);revoked.revoke();
      for (const [index,receiver] of [null,undefined,{},Object.create(doc),Object.create(callee.Document.prototype),author,revoked.proxy,doc.documentElement,owner.document.createDocumentFragment(),owner].entries())check(prefix+'/brand/'+index,()=>typeof get==='function' && typeError(callee,()=>get.call(receiver)) && traps===0);
      check(prefix+'/clone',()=>{const clone=doc.cloneNode(true),expected=docIndex===2 || docIndex===3 ? clone.documentElement : null;return get.call(clone)===expected && (expected===null || expected!==doc.documentElement && expected.ownerDocument===clone);});
    }
    for (const [calleeIndex,callee] of realms.entries()) {
      const doc=owner.document.implementation.createDocument(null,''),get=Object.getOwnPropertyDescriptor(callee.Document.prototype,'rootElement')?.get,prefix=`root/${ownerIndex}/mutate/${calleeIndex}`;
      check(prefix+'/initial',()=>get.call(doc)===null);
      const comment=doc.createComment('leading comment');doc.appendChild(comment);
      for (const [index,[namespace,name,expected]] of [[ns,'svg',true],[ns,'s:svg',true],[ns,'g',false],['urn:other','svg',false],[null,'svg',false],[ns,'SVG',false]].entries()) {
        const element=doc.createElementNS(namespace,name);doc.appendChild(element);
        check(prefix+'/insert/'+index,()=>get.call(doc)===(expected ? element : null));
        const child=doc.createElementNS(ns,'svg');element.appendChild(child);
        check(prefix+'/descendant/'+index,()=>get.call(doc)===(expected ? element : null));
        const replacement=doc.createElementNS(ns,'svg');doc.replaceChild(replacement,element);
        check(prefix+'/replace/'+index,()=>get.call(doc)===replacement && get.call(doc)!==element);
        doc.removeChild(replacement);
        check(prefix+'/remove/'+index,()=>get.call(doc)===null);
      }
      const root=doc.createElementNS(ns,'svg');doc.appendChild(root);
      const other=owner.document.implementation.createDocument(null,'');other.adoptNode(root);other.appendChild(root);
      check(prefix+'/adopt',()=>get.call(doc)===null && get.call(other)===root && root.ownerDocument===other);
    }
  }
  globalThis.__documentSvgRootResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  globalThis.__uiEventResults=globalThis.__documentSvgRootResults;
  return true;
})()
