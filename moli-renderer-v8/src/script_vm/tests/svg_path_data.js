(async () => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed:fn() === true}); }
    catch (error) { checks.push({name, passed:false, error:String(error)}); }
  };
  const ns = 'http://www.w3.org/2000/svg';
  const realms = [window, document.querySelector('iframe').contentWindow];
  const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const errorIn = (realm, fn) => {
    try { fn(); } catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
    return false;
  };
  const rawCases = [
    ['M1 2 3 4L5 6 7 8z', [['M',[1,2]],['L',[3,4]],['L',[5,6]],['L',[7,8]],['Z',[]]]],
    ['m1 2h3v4c1 2 3 4 5 6s7 8 9 10q1 2 3 4t5 6a2 3 45 1 0 4 5z', [['m',[1,2]],['h',[3]],['v',[4]],['c',[1,2,3,4,5,6]],['s',[7,8,9,10]],['q',[1,2,3,4]],['t',[5,6]],['a',[2,3,45,1,0,4,5]],['Z',[]]]],
    ['M1 2H3V4C1 2 3 4 5 6S7 8 9 10Q1 2 3 4T5 6A2 3 45 1 0 4 5', [['M',[1,2]],['H',[3]],['V',[4]],['C',[1,2,3,4,5,6]],['S',[7,8,9,10]],['Q',[1,2,3,4]],['T',[5,6]],['A',[2,3,45,1,0,4,5]]]],
    ['M.5-.25L1e1-2e-1', [['M',[.5,-.25]],['L',[10,Math.fround(-.2)]]]],
    ['M0 0L1. 2L3 4', [['M',[0,0]]]],
    ['M0 0L1e40 2L3 4', [['M',[0,0]]]],
    ['M0 0L3 4X0 0', [['M',[0,0]],['L',[3,4]]]],
    ['M0 0ZL1 2', [['M',[0,0]],['Z',[]],['L',[1,2]]]],
    ['L1 2', []], ['Z', []], ['', []], ['M1', []]
  ];
  const normalizedCases = [
    ['m1 2h3v4l-3 0z', [['M',[1,2]],['L',[4,2]],['L',[4,6]],['L',[1,6]],['Z',[]]]],
    ['M0 0Q3 3 6 0T12 0', [['M',[0,0]],['C',[2,2,4,2,6,0]],['C',[8,-2,10,-2,12,0]]]],
    ['M0 0C1 2 3 4 5 6S7 8 9 10', [['M',[0,0]],['C',[1,2,3,4,5,6]],['C',[7,8,7,8,9,10]]]],
    ['M0 0A0 2 0 0 1 3 4', [['M',[0,0]],['L',[3,4]]]],
    ['M0 0A2 2 0 0 1 0 0', [['M',[0,0]],['L',[0,0]]]],
    ['M1 2Zl3 4m5 6l1 1', [['M',[1,2]],['Z',[]],['L',[4,6]],['M',[9,12]],['L',[10,13]]]]
  ];
  const packed = data => data.map(s => [s.type, s.values]);
  for (const [ownerIndex, owner] of realms.entries()) {
    const docs = [owner.document, owner.document.implementation.createHTMLDocument(''),
      owner.document.implementation.createDocument(ns, 'svg'),
      new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>', 'image/svg+xml')];
    for (const [docIndex, doc] of docs.entries()) for (const [calleeIndex, callee] of realms.entries()) {
      const prefix = `${ownerIndex}/${docIndex}/${calleeIndex}/`;
      const path = doc.createElementNS(ns, 'path');
      const get = callee.SVGPathElement.prototype.getPathData;
      const set = callee.SVGPathElement.prototype.setPathData;
      const put = data => set.call(path, data);
      const read = settings => get.call(path, settings);
      const baseline = () => path.setAttribute('d', 'M 99 99');
      for (const [method, arity] of [['getPathData',0],['setPathData',1]]) check(prefix+method+'/metadata', () => {
        const d = Object.getOwnPropertyDescriptor(callee.SVGPathElement.prototype, method);
        return typeof d.value === 'function' && d.enumerable && d.configurable && d.writable && d.value.name === method && d.value.length === arity && !Object.hasOwn(path,method);
      });
      for (const [d, expected] of rawCases) check(prefix+'raw/'+d, () => {
        path.setAttribute('d',d); return equal(packed(read()),expected);
      });
      for (const [d, expected] of normalizedCases) check(prefix+'normalized/'+d, () => {
        path.setAttribute('d',d); return equal(packed(read({normalize:true})),expected);
      });
      check(prefix+'arc normalization', () => {
        path.setAttribute('d','M6 10A10 10 10 0 0 15 10');
        const data=read({normalize:true}), expected=[8.8305,11.4263,12.1695,11.4263,15,10];
        return data.length===2 && data[0].type==='M' && data[1].type==='C' && data[1].values.every((x,i)=>Math.abs(x-expected[i])<.0005);
      });
      check(prefix+'large arc cubics', () => {
        path.setAttribute('d','M0 0A10 10 0 1 0 20 0');
        const data=read({normalize:true}), end=data.at(-1).values.slice(-2);
        return data.length>2 && data[0].type==='M' && data.slice(1).every(x=>x.type==='C') && Math.abs(end[0]-20)<.0005 && Math.abs(end[1])<.0005;
      });
      check(prefix+'fresh mutable dictionaries', () => {
        path.setAttribute('d','M1 2L3 4');
        const a=read(), b=read(); a[0].type='Z'; a[0].values[0]=900;
        const d=Object.getOwnPropertyDescriptor(b[0],'type');
        return a!==b && a[0]!==b[0] && a[0].values!==b[0].values && equal(packed(b),[['M',[1,2]],['L',[3,4]]]) && d.enumerable && d.configurable && d.writable && equal(Object.keys(b[0]),['type','values']) && equal(packed(read()),packed(b));
      });
      check(prefix+'base attribute ignores CSS d', () => {
        path.setAttribute('d','M1 2h3'); path.style.setProperty('d','path("M0 0L100 100")');
        const result=equal(packed(read()),[['M',[1,2]],['h',[3]]]); path.style.removeProperty('d'); return result;
      });
      for (const settings of [undefined,null,{}, {normalize:false}]) check(prefix+'default settings/'+JSON.stringify(settings), () => {
        path.setAttribute('d','m1 2h3'); return equal(packed(read(settings)),[['m',[1,2]],['h',[3]]]);
      });
      check(prefix+'settings getter once', () => {
        path.setAttribute('d','m1 2h3'); let n=0;
        const data=read({get normalize(){n++;return true;}}); return n===1 && equal(packed(data),[['M',[1,2]],['L',[4,2]]]);
      });
      check(prefix+'settings Boolean object has no coercion', () => {
        path.setAttribute('d','m1 2h3'); let n=0;
        const data=read({normalize:{valueOf(){n++;throw 42;},toString(){n++;throw 42;}}});
        return n===0 && equal(packed(data),[['M',[1,2]],['L',[4,2]]]);
      });
      check(prefix+'settings getter exception', () => {
        const sentinel={}; try {read({get normalize(){throw sentinel;}});} catch(e) {return e===sentinel;} return false;
      });
      check(prefix+'set plain dictionaries', () => put([{type:'M',values:[1,2]},{type:'L',values:[3,4]}])===undefined && path.getAttribute('d')==='M 1 2 L 3 4');
      check(prefix+'generator and typed array values', () => {
        put((function*(){yield {type:'m',values:new Float32Array([1,2])};yield {type:'l',values:new Float64Array([3,4])};yield {type:'z',values:[]};})());
        return path.getAttribute('d')==='m 1 2 l 3 4 Z';
      });
      check(prefix+'inherited dictionary members', () => {
        put([Object.create({type:'M',values:[1,2]})]); return path.getAttribute('d')==='M 1 2';
      });
      check(prefix+'dictionary and numeric conversion order', () => {
        const trace=[];
        put([{get values(){trace.push('values');return [{valueOf(){trace.push('x');return 1;}},{valueOf(){trace.push('y');return 2;}}];},get type(){trace.push('type');return {toString(){trace.push('string');return 'M';}};}}]);
        return trace.join()==='type,string,values,x,y' && path.getAttribute('d')==='M 1 2';
      });
      check(prefix+'arc flags use nonzero truth', () => {
        put([{type:'M',values:[0,0]},{type:'A',values:[-2,3,45,2,-5,4,5]}]);
        return path.getAttribute('d')==='M 0 0 A -2 3 45 1 1 4 5';
      });
      for (const [name, invalid] of [['unknown',{type:'X',values:[]}],['multi-letter',{type:'MM',values:[3,4]}],['arity',{type:'L',values:[3]}],['extra',{type:'L',values:[3,4,5]}],['close-extra',{type:'Z',values:[1]}],['nan',{type:'L',values:[NaN,1]}],['infinity',{type:'L',values:[Infinity,1]}],['overflow',{type:'L',values:[1e40,1]}]]) check(prefix+'valid prefix/'+name, () => {
        put([{type:'M',values:[1,2]},invalid,{type:'L',values:[8,9]}]); return path.getAttribute('d')==='M 1 2';
      });
      for (const input of [[],[{type:'L',values:[1,2]}],[{type:'M',values:[1]}],[{type:'M',values:[NaN,1]}]]) check(prefix+'empty removes/'+JSON.stringify(input), () => {
        baseline();put(input);return !path.hasAttribute('d') && read().length===0;
      });
      for (const input of [undefined,null,{},1,'M',[{}],[{type:'M'}],[{values:[1,2]}],[{type:'M',values:{0:1,1:2,length:2}}],[{type:'M',values:[Symbol(),0]}],[{type:'M',values:[1n,0]}]]) check(prefix+'conversion error/'+String(input), () => {
        baseline(); return errorIn(callee,()=>put(input)) && path.getAttribute('d')==='M 99 99';
      });
      check(prefix+'required sequence argument', () => errorIn(callee,()=>set.call(path)));
      check(prefix+'late dictionary exception is atomic even after invalid segment', () => {
        baseline(); const sentinel={};
        try {put([{type:'X',values:[]},{get type(){throw sentinel;},values:[]}]);} catch(e) {return e===sentinel && path.getAttribute('d')==='M 99 99';} return false;
      });
      check(prefix+'late numeric exception is atomic', () => {
        baseline(); const sentinel={}; let n=0;
        try {put([{type:'M',values:[1,2]},{type:'L',values:[{valueOf(){n++;throw sentinel;}},0]}]);} catch(e) {return e===sentinel && n===1 && path.getAttribute('d')==='M 99 99';} return false;
      });
      check(prefix+'attribute MutationObserver', () => {
        baseline(); const mo=new owner.MutationObserver(()=>{});mo.observe(path,{attributes:true,attributeOldValue:true});
        put([{type:'M',values:[1,2]}]); const first=mo.takeRecords();put([]);const second=mo.takeRecords();mo.disconnect();
        return first.length===1 && second.length===1 && first[0].attributeName==='d' && first[0].oldValue==='M 99 99' && second[0].oldValue==='M 1 2' && !path.hasAttribute('d');
      });
      let traps=0; const author=new owner.Proxy(path,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
      const revoked=owner.Proxy.revocable(path,{});revoked.revoke();
      for (const [index, receiver] of [{},Object.create(callee.SVGPathElement.prototype),Object.create(path),author,revoked.proxy,doc,doc.createElementNS(ns,'rect'),doc.createElement('div')].entries()) {
        check(prefix+'get receiver/'+index, () => {let reads=0;return errorIn(callee,()=>get.call(receiver,{get normalize(){reads++;return true;}})) && reads===0 && traps===0;});
        check(prefix+'set receiver/'+index, () => {let reads=0;return errorIn(callee,()=>set.call(receiver,{get [Symbol.iterator](){reads++;throw 42;}})) && reads===0 && traps===0;});
      }
    }
  }
  globalThis.__svgPathDataResults={complete:true,total:checks.length,passed:checks.filter(x=>x.passed).length,checks};
  globalThis.__uiEventResults=globalThis.__svgPathDataResults;
  return true;
})()
