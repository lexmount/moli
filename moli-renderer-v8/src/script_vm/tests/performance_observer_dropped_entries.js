(async (owner, callee, callbackRealm, tag) => {
  const checks = [];
  const record = (name, passed) => checks.push({name:tag+'/'+name, passed:passed===true});
  const perf = owner.performance;
  for (const mode of ['single','multiple']) {
    const inbox=[], waiters=[];
    let hook;
    const receive = (list, observer, options, realmInfo) => {
      const result={list,observer,options,realmInfo};
      if (waiters.length) waiters.shift()(result); else inbox.push(result);
      if (hook) {const action=hook;hook=undefined;action(observer);}
    };
    const callback=callbackRealm.Function('receive', `return function(list, observer, options) {
      receive(list,observer,options,{
        list: Object.getPrototypeOf(list)===PerformanceObserverEntryList.prototype,
        options: Object.getPrototypeOf(options)===Object.prototype,
        entries: Object.getPrototypeOf(list.getEntries())===Array.prototype
      });
    };`)(receive);
    const observer=new owner.PerformanceObserver(callback);
    const observe=options=>callee.PerformanceObserver.prototype.observe.call(observer,options);
    const initial=mode==='single'?{type:'mark'}:{entryTypes:['mark','resource','mark']};
    const wait=()=>inbox.length?Promise.resolve(inbox.shift()):new Promise(resolve=>waiters.push(resolve));
    let serial=0;
    const mark=()=>perf.mark('drop-contract-'+tag+'-'+mode+'-'+serial++);
    const next=async()=>{const pending=wait();mark();return pending;};
    const descriptor=result=>callbackRealm.Object.getOwnPropertyDescriptor(result.options,'droppedEntriesCount');
    const count=(result,value)=>descriptor(result)?.value===value;
    const absent=result=>!callbackRealm.Object.hasOwn(result.options,'droppedEntriesCount');
    const key=name=>mode+'/'+name;
    try {
      observe(initial);
      if(mode==='single')observe({type:'resource'});
      const first=await next();
      record(key('first-count-zero'),count(first,0));
      record(key('callback-realm'),first.realmInfo.list&&first.realmInfo.options&&first.realmInfo.entries);
      const d=descriptor(first);
      record(key('dictionary-data-descriptor'),d&&d.writable&&d.enumerable&&d.configurable&&!('get'in d));
      record(key('callback-receiver'),first.observer===observer);
      const second=await next();
      record(key('second-count-absent'),absent(second));
      record(key('fresh-dictionary'),second.options!==first.options);
      observe(mode==='single'?{type:'unsupported-drop-entry-type'}:{entryTypes:[]});
      record(key('unsupported-observe-rearms'),count(await next(),0));
      record(key('unsupported-report-once'),absent(await next()));
      observe(initial);
      const taken=mark();
      record(key('take-records-content'),observer.takeRecords().some(entry=>entry===taken));
      await Promise.resolve();
      record(key('empty-delivery-keeps-report'),count(await next(),0));
      let error;
      try {observe({entryTypes:['mark'],buffered:false});}catch(caught){error=caught;}
      record(key('invalid-options-throw'),error instanceof callee.TypeError);
      record(key('invalid-options-do-not-rearm'),absent(await next()));
      const marker={};error=undefined;
      try {observe({get buffered(){throw marker;}});}catch(caught){error=caught;}
      record(key('conversion-exception-identity'),error===marker);
      record(key('conversion-failure-does-not-rearm'),absent(await next()));
      error=undefined;
      try {observe(mode==='single'?{entryTypes:['mark']}:{type:'mark'});}catch(caught){error=caught;}
      record(key('invalid-mode-throw'),error instanceof callee.DOMException&&error.name==='InvalidModificationError');
      record(key('invalid-mode-does-not-rearm'),absent(await next()));
      hook=()=>{observe(initial);mark();};
      const trigger=await next();
      const reentrant=await wait();
      record(key('callback-consumes-before-reobserve'),absent(trigger)&&count(reentrant,0));
      record(key('reentrant-report-once'),absent(await next()));
      observe(initial);observe(initial);
      record(key('repeated-observe-count-once'),count(await next(),0)&&absent(await next()));
      observer.disconnect();observe(initial);
      if(mode==='single')observe({type:'resource'});
      record(key('disconnect-reobserve'),count(await next(),0)&&absent(await next()));
      const prototype=callbackRealm.Object.prototype;
      const original=callbackRealm.Object.getOwnPropertyDescriptor(prototype,'droppedEntriesCount');
      let traps=0;
      callbackRealm.Object.defineProperty(prototype,'droppedEntriesCount',{
        configurable:true,get(){traps++;throw Error('inherited getter');},set(){traps++;throw Error('inherited setter');}
      });
      try {
        observe(initial);
        record(key('inherited-hooks-not-called'),count(await next(),0)&&traps===0);
        const omitted=await next();
        record(key('omission-is-not-undefined-property'),absent(omitted)&&callbackRealm.Object.keys(omitted.options).length===0&&traps===0);
      } finally {
        delete prototype.droppedEntriesCount;
        if(original)callbackRealm.Object.defineProperty(prototype,'droppedEntriesCount',original);
      }
      const globalDescriptor=owner.Object.getOwnPropertyDescriptor(owner,'performance');
      if(globalDescriptor?.configurable){
        let getters=0;
        owner.Object.defineProperty(owner,'performance',{configurable:true,get(){getters++;throw Error('public performance getter');}});
        try {observe(initial);record(key('native-performance-not-public-getter'),count(await next(),0)&&getters===0);}
        finally {owner.Object.defineProperty(owner,'performance',globalDescriptor);}
      }
    } finally {
      callee.PerformanceObserver.prototype.disconnect.call(observer);
      for(const entry of perf.getEntriesByType('mark'))if(entry.name.startsWith('drop-contract-'+tag+'-'+mode+'-'))perf.clearMarks(entry.name);
    }
  }
  return checks;
})
