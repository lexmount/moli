(() => {
  const checks = [];
  const check = (name, fn) => { try {checks.push({name,passed:fn()===true});} catch(error) {checks.push({name,passed:false,error:String(error)});} };
  const ns = 'http://www.w3.org/2000/svg';
  const realms = [window, document.querySelector('iframe').contentWindow];
  const definitions = [
    ['path/missing','path',{},null,0], ['path/empty','path',{d:''},null,0],
    ['path/none','path',{d:'none'},null,0], ['path/invalid-first','path',{d:'L 7 9'},null,0],
    ['path/moveto','path',{d:'M 7 9'},[7,9],0],
    ['path/multiple-moveto','path',{d:'M 7 9 M 12 13'},[7,9],0],
    ['path/zero-line','path',{d:'M 7 9 L 7 9'},[7,9],0],
    ['path/invalid-tail','path',{d:'M 7 9 X 1 2'},[7,9],0],
    ['line/missing','line',{},[0,0],0], ['line/zero','line',{x1:7,y1:9,x2:7,y2:9},[7,9],0],
    ['rect/missing','rect',{},null,0], ['rect/zero','rect',{x:7,y:9,width:0,height:0},null,0],
    ['rect/zero-width','rect',{x:7,y:9,width:0,height:10},[7,9],20],
    ['rect/zero-height','rect',{x:7,y:9,width:10,height:0},[7,9],20],
    ['circle/missing','circle',{},null,0], ['circle/zero','circle',{cx:7,cy:9,r:0},null,0],
    ['ellipse/missing','ellipse',{},null,0], ['ellipse/zero','ellipse',{cx:7,cy:9,rx:0,ry:0},null,0],
    ['ellipse/zero-rx','ellipse',{cx:7,cy:9,rx:0,ry:10},[7,9],40],
    ['ellipse/zero-ry','ellipse',{cx:7,cy:9,rx:10,ry:0},[17,9],40],
    ['polyline/missing','polyline',{},null,0], ['polygon/missing','polygon',{},null,0],
    ['polyline/one-point','polyline',{points:'7 9'},[7,9],0],
    ['polygon/one-point','polygon',{points:'7 9'},[7,9],0],
  ];
  const errorIn = (realm,name,fn) => {try {fn();} catch(error) {return Object.getPrototypeOf(error) === realm[name==='TypeError'?'TypeError':'DOMException'].prototype && error.name===name && (name==='TypeError'||error.code===11);} return false;};
  for (const [ownerIndex,owner] of realms.entries()) {
    const docs=[owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for(const [docIndex,doc] of docs.entries()) {
      const root=doc.createElementNS(ns,'svg');(doc.body||doc.documentElement).appendChild(root);
      for(const [calleeIndex,callee] of realms.entries()) {
        const point=callee.SVGGeometryElement.prototype.getPointAtLength, length=callee.SVGGeometryElement.prototype.getTotalLength;
        for(const [name,tag,attrs,expected,total] of definitions) {
          const prefix=`${ownerIndex}/${docIndex}/${calleeIndex}/${name}`;
          const element=doc.createElementNS(ns,tag);for(const [key,value] of Object.entries(attrs))element.setAttribute(key,value);root.appendChild(element);
          for(const display of ['','none']) {
            element.style.display=display;
            check(prefix+'/length/'+display,()=>total===0?Object.is(length.call(element),0):Math.abs(length.call(element)-total)<1e-6);
            check(prefix+'/point/'+display,()=> {
              if(expected===null)return errorIn(callee,'InvalidStateError',()=>point.call(element,300));
              const value=point.call(element,300);return Object.getPrototypeOf(value)===callee.DOMPoint.prototype&&Math.abs(value.x-expected[0])<1e-7&&Math.abs(value.y-expected[1])<1e-7&&value.z===0&&value.w===1;
            });
          }
          check(prefix+'/detach',()=> {element.remove();if(expected===null)return errorIn(callee,'InvalidStateError',()=>point.call(element,300));const p=point.call(element,300);return Math.abs(p.x-expected[0])<1e-7&&Math.abs(p.y-expected[1])<1e-7;});
          if(expected===null) {
            for(const argument of [NaN,Infinity,-Infinity,Symbol(),1n])check(prefix+'/conversion/'+typeof argument+'/'+String(argument),()=>errorIn(callee,'TypeError',()=>point.call(element,argument)));
            check(prefix+'/missing-argument',()=>errorIn(callee,'TypeError',()=>point.call(element)));
            check(prefix+'/conversion-exception',()=> {const sentinel={};let n=0;try{point.call(element,{valueOf(){n++;throw sentinel;}});}catch(error){return error===sentinel&&n===1;}return false;});
            check(prefix+'/conversion-before-empty',()=> {let n=0;return errorIn(callee,'InvalidStateError',()=>point.call(element,{valueOf(){n++;return 0;}}))&&n===1;});
          }
        }
        const path=doc.createElementNS(ns,'path');root.appendChild(path);
        check(`${ownerIndex}/${docIndex}/${calleeIndex}/conversion-creates-path`,()=> {let n=0;const p=point.call(path,{valueOf(){n++;path.setAttribute('d','M 7 9 L 17 9');return 5;}});return n===1&&p.x===12&&p.y===9;});
        check(`${ownerIndex}/${docIndex}/${calleeIndex}/conversion-removes-path`,()=> {let n=0;return errorIn(callee,'InvalidStateError',()=>point.call(path,{valueOf(){n++;path.removeAttribute('d');return 5;}}))&&n===1;});
        let traps=0;const proxy=new Proxy(path,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});const revoked=Proxy.revocable(path,{});revoked.revoke();
        for(const [index,receiver] of [{},Object.create(callee.SVGGeometryElement.prototype),Object.create(path),proxy,revoked.proxy,doc,doc.createElement('div'),doc.createElementNS(ns,'g')].entries())check(`${ownerIndex}/${docIndex}/${calleeIndex}/brand/${index}`,()=> {let n=0;return errorIn(callee,'TypeError',()=>point.call(receiver,{valueOf(){n++;return 0;}}))&&n===0&&traps===0;});
      }
    }
  }
  globalThis.__svgGeometryResults={complete:true,total:checks.length,passed:checks.filter(x=>x.passed).length,checks};globalThis.__uiEventResults=globalThis.__svgGeometryResults;return true;
})()
