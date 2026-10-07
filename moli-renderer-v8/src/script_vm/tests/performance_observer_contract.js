(function observerContract(owner, callee, tag) {
  const checks = [];
  const record = (name, callback) => {
    try { checks.push({name: 'observer/' + tag + '/' + name, passed: callback() === true}); }
    catch (error) { checks.push({name: 'observer/' + tag + '/' + name, passed: false, error: String(error)}); }
  };
  const observe = callee.PerformanceObserver.prototype.observe;
  const disconnect = callee.PerformanceObserver.prototype.disconnect;
  const make = () => new owner.PerformanceObserver(() => {});
  const run = (options, callback) => {
    const observer = make();
    try { return callback(observer, () => observe.call(observer, options)); }
    finally { disconnect.call(observer); }
  };
  const thrown = callback => { try { callback(); } catch (error) { return error; } };
  const typeError = error => error instanceof callee.TypeError;
  const invalidMode = callback => {
    const error = thrown(callback);
    return error instanceof callee.DOMException && error.name === 'InvalidModificationError';
  };
  for (const kind of ['own', 'inherited', 'proxy']) {
    for (const mode of ['type', 'entries']) {
      record('dictionary-order/' + kind + '/' + mode, () => {
        const trace = [], target = {};
        for (const name of ['buffered', 'durationThreshold', 'entryTypes', 'type']) {
          Object.defineProperty(target, name, {get() {
            trace.push(name);
            if (name === 'durationThreshold' && mode === 'type') return {valueOf() {trace.push('number'); return 16;}};
            if (name === 'type' && mode === 'type') return {toString() {trace.push('string'); return 'mark';}};
            if (name === 'entryTypes' && mode === 'entries') return ['mark'];
            return undefined;
          }});
        }
        const input = kind === 'inherited' ? Object.create(target) : kind === 'proxy' ? new Proxy(target, {}) : target;
        return run(input, (_, call) => {
          call();
          const expected = mode === 'type' ? ['buffered','durationThreshold','number','entryTypes','type','string'] : ['buffered','durationThreshold','entryTypes','type'];
          return trace.join() === expected.join();
        });
      });
    }
    for (const key of ['buffered', 'durationThreshold', 'entryTypes', 'type']) {
      record('getter-throws/' + kind + '/' + key, () => {
        const trace = [], sentinel = {}, target = {};
        const names = ['buffered','durationThreshold','entryTypes','type'];
        for (const name of names) Object.defineProperty(target, name, {get() {
          trace.push(name); if (name === key) throw sentinel; return undefined;
        }});
        const value = kind === 'inherited' ? Object.create(target) : kind === 'proxy' ? new Proxy(target, {}) : target;
        return run(value, (_, call) => thrown(call) === sentinel && trace.join() === names.slice(0,names.indexOf(key)+1).join());
      });
    }
  }
  for (const mode of ['type', 'entries', 'invalid']) {
    for (const [label, value] of [['nan',NaN], ['positive-infinity',Infinity], ['negative-infinity',-Infinity], ['symbol',Symbol()], ['bigint',1n]]) {
      record('restricted-double/' + mode + '/' + label, () => {
        let later = 0;
        return run({durationThreshold:value, get entryTypes() {later++; return mode === 'entries' ? ['mark'] : undefined;}, get type() {later++; return mode === 'type' ? 'mark' : undefined;}}, (_,call) => typeError(thrown(call)) && later === 0);
      });
    }
    record('number-throws/' + mode, () => {
      const sentinel = {}; let later = 0;
      return run({durationThreshold:{[Symbol.toPrimitive]() {throw sentinel;}}, get entryTypes() {later++;return mode === 'entries' ? ['mark'] : undefined;}, get type() {later++;return mode === 'type' ? 'mark' : undefined;}}, (_,call) => thrown(call) === sentinel && later === 0);
    });
  }
  for (const [name, value] of [['absent',undefined],['null',null],['false',false],['true',true],['zero',0],['fraction',0.25],['negative',-10],['large',1e100],['string','16'],['object',{valueOf(){return 16;}}]]) {
    record('type-threshold/' + name, () => run({type:'mark',durationThreshold:value}, (_,call) => {call();return true;}));
    record('entries-threshold/' + name, () => run({entryTypes:['mark'],durationThreshold:value}, (_,call) => value === undefined ? (call(),true) : typeError(thrown(call))));
  }
  for (const [name,value] of [['absent',undefined],['null',null],['false',false],['true',true],['zero',0],['empty-string',''],['object',{}]]) {
    record('entries-buffered/' + name, () => run({entryTypes:['mark'],buffered:value}, (_,call) => value === undefined ? (call(),true) : typeError(thrown(call))));
  }
  record('boolean-no-primitive', () => {
    let calls = 0;
    return run({type:'mark',buffered:{[Symbol.toPrimitive]() {calls++;throw Error('must not convert');}}},(_,call) => {call();return calls === 0;});
  });
  for (const [name, first, second] of [
    ['empty-multiple',{entryTypes:[]},{type:'mark'}],
    ['unsupported-multiple',{entryTypes:['unknown-observer-type']},{type:'mark'}],
    ['unsupported-single',{type:'unknown-observer-type'},{entryTypes:['mark']}],
    ['supported-multiple',{entryTypes:['mark']},{type:'measure'}],
    ['supported-single',{type:'mark'},{entryTypes:['measure']}],
  ]) {
    for (const disconnected of [false,true]) record('mode/' + name + '/' + disconnected, () => {
      const observer = make();
      try { observe.call(observer,first); if (disconnected) disconnect.call(observer); return invalidMode(() => observe.call(observer,second)); }
      finally { disconnect.call(observer); }
    });
  }
  for (const [name, first] of [
    ['empty-options',{}],['mixed-type',{entryTypes:['mark'],type:'mark'}],['mixed-buffered',{entryTypes:['mark'],buffered:false}],['mixed-threshold',{entryTypes:['mark'],durationThreshold:0}],
  ]) for (const second of [{type:'mark'},{entryTypes:['mark']}]) record('failed-options-do-not-select-mode/' + name + '/' + ('type' in second), () => {
    const observer = make();
    try { if (!typeError(thrown(() => observe.call(observer,first)))) return false; observe.call(observer,second); return true; }
    finally { disconnect.call(observer); }
  });
  record('conversion-failure-does-not-select-mode', () => {
    const observer = make(), sentinel = {};
    try {
      if (thrown(() => observe.call(observer,{type:'mark',durationThreshold:{valueOf(){throw sentinel;}}})) !== sentinel) return false;
      observe.call(observer,{entryTypes:['mark']}); return true;
    } finally { disconnect.call(observer); }
  });
  for (const [name, forge] of [
    ['ordinary',() => ({})], ['prototype',() => Object.create(owner.PerformanceObserver.prototype)],
    ['inherited-native', value => Object.create(value)], ['author-proxy', value => new Proxy(value,{})],
    ['revoked',value => {const proxy=Proxy.revocable(value,{});proxy.revoke();return proxy.proxy;}],
  ]) record('receiver/' + name, () => {
    const observer = make(); let gets=0;
    try { const error=thrown(() => observe.call(forge(observer),new Proxy({},{get(){gets++;return undefined;}})));return typeError(error) && gets===0; }
    finally { disconnect.call(observer); }
  });
  // Subscription changes are tested in the observer's own realm so the marks
  // have the same relevant global as the observer.
  if (owner === callee) {
    for (const kind of ['single','multiple']) record('disconnect-clears-subscriptions/' + kind, () => {
      const observer = make(), unique='observer-contract-' + tag + '-' + kind;
      try {
        observe.call(observer,kind==='single' ? {type:'mark'} : {entryTypes:['mark']});
        owner.performance.mark(unique+'-before');
        disconnect.call(observer);
        if (observer.takeRecords().length !== 0) return false;
        observe.call(observer,kind==='single' ? {type:'measure'} : {entryTypes:['measure']});
        owner.performance.mark(unique+'-mark');owner.performance.measure(unique+'-measure');
        const entries=observer.takeRecords();
        return entries.length===1 && entries[0].name===unique+'-measure' && entries[0].entryType==='measure';
      } finally {disconnect.call(observer);owner.performance.clearMarks(unique+'-before');owner.performance.clearMarks(unique+'-mark');owner.performance.clearMeasures(unique+'-measure');}
    });
    record('unsupported-call-preserves-registration', () => {
      const observer=make(),unique='observer-contract-'+tag+'-registration';
      try {observe.call(observer,{type:'mark'});observe.call(observer,{type:'unsupported-observer-type'});owner.performance.mark(unique);return observer.takeRecords().some(entry=>entry.name===unique);}
      finally {disconnect.call(observer);owner.performance.clearMarks(unique);}
    });
  }
  return checks;
})
