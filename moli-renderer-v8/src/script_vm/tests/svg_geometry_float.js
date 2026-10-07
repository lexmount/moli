(() => {
  const checks = [], ns = 'http://www.w3.org/2000/svg';
  const check = (name,fn) => {try {checks.push({name,passed:fn()===true});} catch(error) {checks.push({name,passed:false,error:String(error)});}};
  const near = (a,b) => Math.abs(a-b) <= Math.max(1e-12,Math.abs(b)*2e-14);
  const realms = [window,document.querySelector('iframe').contentWindow];
  const values = [0,-0,1,1+2**-24,1+3*2**-24,16777217,16777219,1e-46,-1,1e20,-1e20,Math.fround(3.4028234e38),null,true,'1.0000001788139343'];
  const invalid = [undefined,NaN,Infinity,-Infinity,Number.MAX_VALUE,-Number.MAX_VALUE,2**128,-(2**128),Symbol(),1n];
  const typeError = (realm,fn) => {try {fn();} catch(error) {return Object.getPrototypeOf(error)===realm.TypeError.prototype;} return false;};
  for (const [ownerIndex,owner] of realms.entries()) {
    const docs = [owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for (const [docIndex,doc] of docs.entries()) {
      const root=doc.createElementNS(ns,'svg');root.setAttribute('width','200');root.setAttribute('height','100');(doc.body||doc.documentElement).appendChild(root);
      for (const [calleeIndex,callee] of realms.entries()) {
        const prefix=`float/${ownerIndex}/${docIndex}/${calleeIndex}`;
        const point=callee.SVGGeometryElement.prototype.getPointAtLength, total=callee.SVGGeometryElement.prototype.getTotalLength;
        const path=doc.createElementNS(ns,'path');path.setAttribute('d','M0 0L134217728 0');root.appendChild(path);
        for (const [index,value] of values.entries()) {
          check(prefix+'/distance/'+index,()=>{const p=point.call(path,value);return near(p.x,Math.min(134217728,Math.max(0,Math.fround(Number(value)))))&&p.y===0;});
          check(prefix+'/object-distance/'+index,()=>{let n=0;const p=point.call(path,{valueOf(){n++;return value;}});return n===1&&near(p.x,Math.min(134217728,Math.max(0,Math.fround(Number(value)))))&&p.y===0;});
        }
        const empty=doc.createElementNS(ns,'path');root.appendChild(empty);
        for (const [index,value] of invalid.entries()) {
          for (const [tag,element] of [['path',path],['empty',empty]]) {
            check(prefix+'/'+tag+'/invalid/'+index,()=>typeError(callee,()=>point.call(element,value)));
          }
        }
        check(prefix+'/arity',()=>typeError(callee,()=>point.call(path)));
        check(prefix+'/conversion-exception',()=>{const sentinel={};let n=0;try {point.call(path,{valueOf(){n++;throw sentinel;}});} catch(error) {return error===sentinel&&n===1;}return false;});
        check(prefix+'/conversion-mutates-path',()=>{let n=0;const p=point.call(path,{valueOf(){n++;path.setAttribute('d','M7 9L134217735 9');return 16777217;}});return n===1&&near(p.x,16777223)&&p.y===9;});
        for (const [index,value] of [0.1,1.00000007,16777217,16777219,1000.00001].entries()) {
          const line=doc.createElementNS(ns,'line');line.setAttribute('x2',String(value));root.appendChild(line);
          check(prefix+'/total-float/'+index,()=>Object.is(total.call(line),Math.fround(value)));
          line.remove();
        }
        let conversions=0,traps=0;
        const author=new Proxy(path,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}}),revoked=Proxy.revocable(path,{});revoked.revoke();
        for (const [index,receiver] of [{},Object.create(path),Object.create(callee.SVGGeometryElement.prototype),author,revoked.proxy,doc.createElementNS(ns,'g')].entries()) {
          check(prefix+'/brand/'+index,()=>typeError(callee,()=>point.call(receiver,{valueOf(){conversions++;return Number.MAX_VALUE;}}))&&conversions===0&&traps===0);
        }
        path.remove();empty.remove();
      }
      root.remove();
    }
  }
  globalThis.__svgFloatResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};globalThis.__uiEventResults=globalThis.__svgFloatResults;return true;
})()
