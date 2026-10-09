(function reportingObserverContract(owner, callee, tag) {
  const checks = [];
  const record = (name, test) => {
    try { checks.push({name:tag+'/'+name, passed:test() === true}); }
    catch (error) { checks.push({name:tag+'/'+name, passed:false, error:String(error)}); }
  };
  const thrown = test => { try { test(); } catch (error) { return error; } };
  const C = callee.ReportingObserver;
  const make = () => new owner.ReportingObserver(() => {});
  record('constructor-shape', () => C.length === 1 && C.name === 'ReportingObserver');
  record('requires-new', () => thrown(() => C(() => {})) instanceof callee.TypeError);
  record('missing-callback', () => thrown(() => new C()) instanceof callee.TypeError);
  for (const [name, callback] of [['undefined',undefined],['null',null],['object',{}],['number',2],['string','cb'],['symbol',Symbol()]]) {
    record('callback/'+name, () => {
      let reads=0;
      const error=thrown(() => new C(callback, {get buffered(){reads++;},get types(){reads++;}}));
      return error instanceof callee.TypeError && reads===0;
    });
  }
  for (const [name, options] of [['undefined',undefined],['null',null],['empty',{}],['empty-types',{types:[]}],['unknown',{types:['unknown']}],['lone-surrogate',{types:['\ud800']}],['set',{types:new Set(['csp-violation'])}]]) {
    record('options/'+name, () => new C(() => {}, options) instanceof C);
  }
  for (const [name, options] of [['boolean',true],['number',1],['string','types'],['symbol',Symbol()],['bigint',1n]]) {
    record('dictionary/'+name, () => thrown(() => new C(() => {}, options)) instanceof callee.TypeError);
  }
  for (const kind of ['own','inherited','proxy']) {
    record('dictionary-order/'+kind, () => {
      const trace=[], options={get buffered(){trace.push('buffered');return {valueOf(){throw Error('Boolean conversion called user code');}};},get types(){trace.push('types');return [{toString(){trace.push('string');return 'csp-violation';}}];}};
      const value=kind==='inherited'?Object.create(options):kind==='proxy'?new Proxy(options,{}):options;
      new C(() => {},value);
      return trace.join()==='buffered,types,string';
    });
    for(const key of ['buffered','types']) record('dictionary-throws/'+kind+'/'+key, () => {
      const trace=[], sentinel={}, options={};
      for(const name of ['buffered','types']) Object.defineProperty(options,name,{get(){trace.push(name);if(name===key)throw sentinel;return undefined;}});
      const value=kind==='inherited'?Object.create(options):kind==='proxy'?new Proxy(options,{}):options;
      return thrown(() => new C(() => {},value))===sentinel && trace.join()===(key==='buffered'?'buffered':'buffered,types');
    });
  }
  for(const [name, types] of [['null',null],['number',1],['string','csp-violation'],['object',{}],['symbol-member',[Symbol()]],['null-iterator',{[Symbol.iterator]:null}]]) {
    record('sequence/'+name, () => thrown(() => new C(() => {},{types})) instanceof callee.TypeError);
  }
  record('sequence-throws', () => {
    const sentinel={};return thrown(() => new C(() => {},{types:{[Symbol.iterator](){throw sentinel;}}}))===sentinel;
  });
  record('callback-proxy-no-traps', () => {
    let reads=0;const cb=new Proxy(() => {},{get(){reads++;throw Error('callback property read');},apply(){throw Error('called during construction');}});
    new C(cb);return reads===0;
  });
  const methods=['observe','disconnect','takeRecords'];
  for(const key of methods) {
    const method=C.prototype[key];
    record('descriptor/'+key, () => {const d=Object.getOwnPropertyDescriptor(C.prototype,key);return method.length===0 && method.name===key && d.writable && d.enumerable && d.configurable;});
    for(const [name, forge] of [['ordinary',()=>({})],['prototype',()=>Object.create(owner.ReportingObserver.prototype)],['inherited',v=>Object.create(v)],['proxy',v=>new Proxy(v,{})],['revoked',v=>{const p=Proxy.revocable(v,{});p.revoke();return p.proxy;}]]) {
      record('receiver/'+key+'/'+name, () => {
        let reads=0;const real=make();
        const error=thrown(() => method.call(forge(real),new Proxy({},{get(){reads++;throw Error('ignored argument read');}})));
        return error instanceof callee.TypeError && reads===0;
      });
    }
    record('genuine-cross-realm/'+key, () => {
      const real=make(), ignored=new Proxy({},{get(){throw Error('ignored argument read');}});
      try {const value=method.call(real,ignored);return key==='takeRecords'?Array.isArray(value)&&value.length===0:value===undefined;}
      finally {owner.ReportingObserver.prototype.disconnect.call(real);}
    });
  }
  record('fresh-array', () => {const real=make();const a=C.prototype.takeRecords.call(real);a.push('author');const b=C.prototype.takeRecords.call(real);return a!==b && b.length===0;});
  record('return-array-callee-realm', () => C.prototype.takeRecords.call(make()) instanceof callee.Array);
  record('idempotent-subscription', () => {
    let callbacks=0;const real=new owner.ReportingObserver(()=>callbacks++);
    for(let i=0;i<3;i++) C.prototype.observe.call(real);
    for(let i=0;i<3;i++) C.prototype.disconnect.call(real);
    C.prototype.observe.call(real);C.prototype.disconnect.call(real);
    return callbacks===0 && C.prototype.takeRecords.call(real).length===0;
  });
  return checks;
})
