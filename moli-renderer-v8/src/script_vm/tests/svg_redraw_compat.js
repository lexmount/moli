(() => {
  const checks = [], ns = 'http://www.w3.org/2000/svg';
  const check = (name, fn) => {try {checks.push({name, passed: fn() === true});} catch(error) {checks.push({name, passed: false, error: String(error)});}};
  const realms = [window, document.querySelector('iframe').contentWindow];
  const methods = [['suspendRedraw',1,1],['unsuspendRedraw',1,undefined],['unsuspendRedrawAll',0,undefined],['forceRedraw',0,undefined]];
  const values = [undefined,null,false,true,0,-0,1,-1,1.9,-1.9,2**32,2**32+1,-(2**32)-1,NaN,Infinity,-Infinity,'12','invalid',Number.MAX_VALUE];
  const typeError = (realm, fn) => {try {fn();} catch(error) {return Object.getPrototypeOf(error) === realm.TypeError.prototype;} return false;};
  for (const [ownerIndex, owner] of realms.entries()) {
    const docs = [owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for (const [docIndex, doc] of docs.entries()) {
      const root = doc.createElementNS(ns,'svg');root.setAttribute('viewBox','0 0 20 30');(doc.body || doc.documentElement).appendChild(root);
      const line = doc.createElementNS(ns,'line');line.setAttribute('x2','10');root.appendChild(line);
      for (const [calleeIndex, callee] of realms.entries()) {
        const prefix = `redraw/${ownerIndex}/${docIndex}/${calleeIndex}`;
        for (const [name, length, expected] of methods) {
          const fn = callee.SVGSVGElement.prototype[name];
          check(prefix+'/'+name+'/descriptor', () => {
            const d = Object.getOwnPropertyDescriptor(callee.SVGSVGElement.prototype,name);
            return !!d && d.enumerable && d.configurable && d.writable && typeof d.value === 'function' && d.value.name === name && d.value.length === length;
          });
          check(prefix+'/'+name+'/callable-only', () => typeError(window, () => new fn(1)));
          check(prefix+'/'+name+'/missing-argument', () => length === 1 ? typeof fn === 'function' && typeError(callee, () => fn.call(root)) : fn.call(root) === expected);
          if (length === 1) {
            for (const [index,value] of values.entries()) {
              check(prefix+'/'+name+'/value/'+index, () => fn.call(root,value) === expected);
              check(prefix+'/'+name+'/convert/'+index, () => {let n=0;const result=fn.call(root,{valueOf(){n++;return value;}});return result === expected && n === 1;});
            }
            for (const [index,value] of [1n,Symbol()].entries()) check(prefix+'/'+name+'/invalid-number/'+index, () => typeof fn === 'function' && typeError(callee, () => fn.call(root,value)));
            check(prefix+'/'+name+'/exception-identity', () => {const sentinel={};let n=0;try {fn.call(root,{valueOf(){n++;throw sentinel;}});} catch(error) {return error === sentinel && n === 1;}return false;});
            check(prefix+'/'+name+'/primitive-hint', () => {const hints=[];return fn.call(root,{[Symbol.toPrimitive](hint){hints.push(hint);return 5;}}) === expected && String(hints) === 'number';});
            check(prefix+'/'+name+'/conversion-mutation', () => {let n=0;const result=fn.call(root,{valueOf(){n++;root.setAttribute('data-converted',name);return 42;}});return result === expected && n === 1 && root.getAttribute('data-converted') === name;});
          }
          check(prefix+'/'+name+'/extra-arguments', () => {let n=0;const bad={valueOf(){n++;throw 1;},toString(){n++;throw 2;}};return fn.call(root,...(length ? [0,bad] : [bad])) === expected && n === 0;});
          check(prefix+'/'+name+'/no-dom-effect', () => {const before=root.outerHTML,lengthBefore=line.getTotalLength();const observer=new owner.MutationObserver(()=>{});observer.observe(root,{attributes:true,childList:true,characterData:true,subtree:true});try {return fn.call(root,0) === expected && root.outerHTML === before && observer.takeRecords().length === 0 && line.getTotalLength() === lengthBefore;} finally {observer.disconnect();}});
          let traps=0;
          const handler={get(){traps++;throw 1;},getPrototypeOf(){traps++;throw 2;}};
          const author=new owner.Proxy(root,handler),revoked=owner.Proxy.revocable(root,handler);revoked.revoke();
          for (const [index,receiver] of [null,undefined,{},Object.create(root),Object.create(callee.SVGSVGElement.prototype),author,revoked.proxy,doc,doc.createElementNS(ns,'g'),doc.createElement('svg')].entries()) {
            check(prefix+'/'+name+'/brand/'+index, () => {let n=0;return typeof fn === 'function' && typeError(callee, () => fn.call(receiver,{valueOf(){n++;return 0;}})) && n === 0 && traps === 0;});
          }
          check(prefix+'/'+name+'/detached', () => {root.remove();const result=fn.call(root,999);(doc.body || doc.documentElement).appendChild(root);return result === expected;});
        }
        check(prefix+'/constant-handle', () => root.suspendRedraw(1) === 1 && root.suspendRedraw(2) === 1 && root.unsuspendRedraw(999) === undefined && root.unsuspendRedrawAll() === undefined && root.forceRedraw() === undefined && root.suspendRedraw(0) === 1);
      }
      root.remove();
    }
  }
  globalThis.__svgRedrawResults = {complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  globalThis.__uiEventResults = globalThis.__svgRedrawResults;
  return true;
})()
