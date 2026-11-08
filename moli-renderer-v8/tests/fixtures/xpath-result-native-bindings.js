(() => {
  const rows = [];
  let traps = 0;
  const check = (name, body) => {
    traps = 0;
    try {if (!body()) throw Error('false'); rows.push({name,passed:true});}
    catch(error) {rows.push({name,passed:false,error:String(error)});}
  };
  const other = document.querySelector('#child').contentWindow;
  const attributes = ['resultType','invalidIteratorState','numberValue','stringValue','booleanValue','singleNodeValue','snapshotLength'];
  for (const [ci,callee] of [globalThis,other].entries()) for (const [oi,owner] of [globalThis,other].entries())
  for (const flavor of ['live','html','xml']) {
    const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
      owner.document.implementation.createHTMLDocument('xpath') : owner.document.implementation.createDocument(null,'r');
    const root = doc.createElement('root'), a = doc.createElement('a'), b = doc.createElement('b');
    root.appendChild(a); root.appendChild(b); (doc.body || doc.documentElement).appendChild(root);
    const prefix = `${ci}/${oi}/${flavor}`, proto = callee.XPathResult.prototype;
    const descriptor = name => Object.getOwnPropertyDescriptor(proto,name);
    const typeError = body => {try {body();return false;} catch(error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}};
    const invalidState = body => {try {body();return false;} catch(error) {return error.name === 'InvalidStateError' && error.code === 11 && Object.getPrototypeOf(error) === callee.DOMException.prototype;}};
    const evaluate = (expression,type,context=root) => doc.evaluate(expression,context,null,type,null);
    const make = type => evaluate(type===1?'7':type===2?"'value'":type===3?'true()':'child::*',type);
    const genuine = make(7);
    const handler = {get() {traps++;throw Error('get trap');},getPrototypeOf() {traps++;throw Error('prototype trap');}};
    const revoked = Proxy.revocable(genuine,handler); revoked.revoke();
    const invalid = [null,undefined,0,true,'',Symbol(),{},Object.create(proto),Object.create(genuine),new Proxy(genuine,handler),revoked.proxy,root,new callee.Event('test')];
    for (const name of attributes.concat(['iterateNext','snapshotItem'])) {
      for (const [ri,receiver] of invalid.entries()) check(`${prefix}/${name}/receiver/${ri}`,() => {
        let converted = 0;
        const index = {valueOf() {converted++;throw Error('index conversion');}};
        const body = attributes.includes(name) ? () => descriptor(name).get.call(receiver) : () => proto[name].call(receiver,index);
        return typeError(body) && converted === 0 && traps === 0;
      });
      check(`${prefix}/${name}/descriptor`,() => {
        const d = descriptor(name);
        if (!d.enumerable || !d.configurable) return false;
        return attributes.includes(name) ? typeof d.get === 'function' && d.get.length === 0 && d.set === undefined :
          d.writable && typeof d.value === 'function' && d.value.length === (name === 'snapshotItem' ? 1 : 0);
      });
    }
    for (let type=1;type<=9;type++) {
      const result = make(type), name = `${prefix}/type/${type}`;
      check(name+'/state',() => descriptor('resultType').get.call(result) === type && descriptor('invalidIteratorState').get.call(result) === false);
      const supported = {numberValue:type===1,stringValue:type===2,booleanValue:type===3,singleNodeValue:type===8||type===9,snapshotLength:type===6||type===7};
      const expected = {numberValue:7,stringValue:'value',booleanValue:true,singleNodeValue:a,snapshotLength:2};
      for (const member of Object.keys(supported)) check(name+'/'+member,() =>
        supported[member] ? descriptor(member).get.call(result) === expected[member] : typeError(() => descriptor(member).get.call(result)));
      if (type===4 || type===5) check(name+'/iteration',() => proto.iterateNext.call(result) === a && proto.iterateNext.call(result) === b && proto.iterateNext.call(result) === null && proto.iterateNext.call(result) === null);
      else check(name+'/wrong-iterator',() => typeError(() => proto.iterateNext.call(result)));
      if (type===6 || type===7) {
        for (const [ii,index] of [0,1,2,-1,4294967296,4294967297,NaN,Infinity,undefined,null,-0,1.9].entries())
          check(name+`/index/${ii}`,() => proto.snapshotItem.call(result,index) === ([a,b,null,null,a,b,a,a,a,a,a,b][ii]));
      } else check(name+'/wrong-snapshot',() => typeError(() => proto.snapshotItem.call(result,0)));
      check(name+'/missing-index',() => typeError(() => proto.snapshotItem.call(result)));
      check(name+'/throwing-index',() => {
        const sentinel = {}; let count = 0;
        try {proto.snapshotItem.call(result,{valueOf() {count++;throw sentinel;}}); return false;}
        catch(error) {return error === sentinel && count === 1;}
      });
      check(name+'/invalid-index',() => {
        let count = 0;
        return typeError(() => proto.snapshotItem.call(result,{valueOf() {count++;return Symbol();}})) && count === 1;
      });
      check(name+'/reentrant-index',() => {
        let count = 0; const inserted = doc.createElement('conversion');
        let value, error;
        try {value = proto.snapshotItem.call(result,{valueOf() {count++;root.appendChild(inserted);return 0;}});} catch(caught) {error = caught;}
        const ok = count === 1 && inserted.parentNode === root &&
          (type===6 || type===7 ? error === undefined && value === a : error && Object.getPrototypeOf(error) === callee.TypeError.prototype);
        inserted.remove(); return ok;
      });
      check(name+'/private-state',() => {
        Object.defineProperty(result,'resultType',{configurable:true,get() {throw Error('public resultType');}});
        Object.defineProperty(result,'invalidIteratorState',{configurable:true,get() {throw Error('public invalidIteratorState');}});
        const ok = descriptor('resultType').get.call(result) === type;
        delete result.resultType; delete result.invalidIteratorState; return ok;
      });
    }
    for (const type of [4,5,6,7]) check(`${prefix}/mutation/${type}`,() => {
      const result = make(type), was = descriptor('invalidIteratorState').get.call(result);
      const initial = type===4 || type===5 ? proto.iterateNext.call(result) : proto.snapshotItem.call(result,0);
      a.setAttribute('changed',String(type));
      if (type===4 || type===5) return !was && initial === a && descriptor('invalidIteratorState').get.call(result) && invalidState(() => proto.iterateNext.call(result));
      return !was && initial === a && !descriptor('invalidIteratorState').get.call(result) && descriptor('snapshotLength').get.call(result) === 2 && proto.snapshotItem.call(result,0) === a;
    });
    check(`${prefix}/snapshot/removal`,() => {
      const snapshot = make(7); a.remove();
      return descriptor('snapshotLength').get.call(snapshot) === 2 && proto.snapshotItem.call(snapshot,0) === a && proto.snapshotItem.call(snapshot,1) === b && !descriptor('invalidIteratorState').get.call(snapshot);
    });
    root.remove();
  }
  globalThis.__uiEventResults = {complete:true,total:rows.length,passed:rows.filter(row=>row.passed).length,rows};
  return rows.every(row=>row.passed);
})()
