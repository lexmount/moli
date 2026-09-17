globalThis.recordChecks = [];
async function recordProbe(prefix = 'get-all-records-' + Math.random()) {
  const checks = globalThis.recordChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: JSON.stringify(actual)});
  const equal = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), actual);
  const throws = (label, call, name, identity) => {
    try { call(); check(label, false, 'accepted'); }
    catch (error) { check(label, error.name === name && (!identity || error === identity), error.name); }
  };
  const request = r => new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const done = tx => new Promise((resolve, reject) => {
    tx.oncomplete = resolve;
    tx.onabort = () => reject(tx.error);
  });
  check('IDBRecord exposed', typeof IDBRecord === 'function');
  for (const prototype of [IDBObjectStore.prototype, IDBIndex.prototype]) {
    for (const method of ['getAll', 'getAllKeys', 'getAllRecords']) {
      check(prototype.constructor.name + '.' + method + ' length', typeof prototype[method] === 'function' && prototype[method].length === 0);
    }
  }
  if (typeof IDBRecord !== 'function') return {state:'fail', checks};
  throws('illegal constructor', () => new IDBRecord(), 'TypeError');
  throws('illegal call', () => IDBRecord(), 'TypeError');
  const eager = [];
  const open = indexedDB.open(prefix, 1);
  open.onupgradeneeded = () => {
    const db = open.result, store = db.createObjectStore('s');
    store.createIndex('i', 'group');
    store.createIndex('multi', 'tags', {multiEntry:true});
    for (const [id, group, tags] of [[1,'a',['x','y']], [2,'a',['x']], [3,'b',['z']]]) store.put({id, group, tags}, id);
    const complex = db.createObjectStore('complex');
    complex.createIndex('i', 'index');
    const value = {index:[new Date(20), new Uint8Array([4,5]), ['\ud800']], payload:{name:'original'}};
    value.self = value;
    complex.put(value, [new Date(10), new Uint8Array([1,2]), ['\udfff']]);
    eager.push(request(store.getAllRecords({direction:'prev',count:2})).then(rows => equal('upgrade eager records', rows.map(r => r.primaryKey), [3,2])));
    eager.push(request(store.index('i').getAllRecords({direction:'prevunique'})).then(rows => equal('upgrade eager unique', rows.map(r => r.primaryKey), [3,1])));
  };
  const db = await request(open);
  await Promise.all(eager);
  try {
    const pending=[];
    for (const sourceName of ['store', 'index']) {
      for (const method of ['getAll', 'getAllKeys', 'getAllRecords']) {
        const tx = db.transaction('s'), store=tx.objectStore('s'), source=sourceName==='store' ? store : store.index('i');
        const label=sourceName+'.'+method;
        const primary = rows => method==='getAll' ? rows.map(v=>v.id) : method==='getAllKeys' ? rows : rows.map(v=>v.primaryKey);
        for (const [direction, expected] of [['next',[1,2,3]], ['prev',[3,2,1]], ['nextunique',sourceName==='store'?[1,2,3]:[1,3]], ['prevunique',sourceName==='store'?[3,2,1]:[3,1]]]) {
          const r=source[method]({direction, count:0}, 1);
          check(label+':source/transaction:'+direction, r.source===source && r.transaction===tx);
          pending.push(request(r).then(rows=>equal(label+':'+direction,primary(rows),expected)));
        }
        pending.push(request(source[method]({direction:'prev',count:1})).then(rows=>equal(label+':limit',primary(rows),[3])));
        pending.push(request(source[method]({query:sourceName==='store'?2:'a'})).then(rows=>equal(label+':query',primary(rows),sourceName==='store'?[2]:[1,2])));
        pending.push(request(source[method]({query:IDBKeyRange.bound(sourceName==='store'?1:'a',sourceName==='store'?2:'a'),direction:'prev',count:1})).then(rows=>equal(label+':range',primary(rows),[2])));
        pending.push(request(source[method](Object.create({count:1,direction:'prev'}))).then(rows=>equal(label+':inherited-options',primary(rows),[3])));
        pending.push(request(source[method](null)).then(rows=>equal(label+':null',primary(rows),[1,2,3])));
        const order=[];
        const options={get count(){order.push('count');return {valueOf(){order.push('count.valueOf');return 0}}}, get direction(){order.push('direction');return {toString(){order.push('direction.toString');return 'prev'}}}, get query(){order.push('query');return undefined}};
        const second={valueOf(){order.push('second');return 1}};
        pending.push(request(source[method](options,second)).then(rows=>equal(label+':dictionary overrides positional count',primary(rows),[3,2,1])));
        equal(label+':conversion order',order, (method==='getAllRecords'?[]:['second']).concat(['count','count.valueOf','direction','direction.toString','query']));
        for (const [option, value] of [['count',-1],['count',NaN],['count',Infinity],['count',4294967296],['count',1n],['direction','sideways'],['direction',null]]) throws(label+':invalid '+option+':'+String(value),()=>source[method]({[option]:value}),'TypeError');
        for (const member of ['count','direction','query']) {
          const sentinel=new Error(member), seen=[];
          const bad=Object.fromEntries(['count','direction','query'].map(k=>[k,undefined]));
          Object.defineProperty(bad, member,{get(){seen.push(member);throw sentinel}});
          throws(label+':throw '+member,()=>source[method](bad),'Error',sentinel);
          equal(label+':read '+member+' once',seen,[member]);
        }
        const revocable=Proxy.revocable(source,{});revocable.revoke();
        for (const [name, receiver] of [['fake',Object.create(Object.getPrototypeOf(source))],['proxy',new Proxy(source,{})],['revoked',revocable.proxy]]) {
          let reads=0;
          throws(label+':brand '+name,()=>source[method].call(receiver,{get count(){reads++;return 1}}),'TypeError');
          equal(label+':brand before conversion '+name,reads,0);
        }
        const badKey=[1];Object.defineProperty(badKey,0,{get(){throw new URIError('key')}});
        throws(label+':query exception',()=>source[method]({query:badKey}),'URIError');
        for(const invalid of [NaN,new Date(NaN),[undefined]]) throws(label+':invalid key '+String(invalid),()=>source[method]({query:invalid}),'DataError');
        if(method!=='getAllRecords') {
          pending.push(request(source[method](undefined,0)).then(rows=>equal(label+':positional zero',primary(rows),[1,2,3])));
          let reads=0;throws(label+':bad positional count',()=>source[method]({get count(){reads++;return 1}},-1),'TypeError');equal(label+':positional before dictionary',reads,0);
          for(const invalid of [new Date(NaN),[undefined]]) throws(label+':invalid keylike argument '+String(invalid),()=>source[method](invalid),'DataError');
        } else {
          for(const primitive of [1,'a',true,Symbol('s')]) throws(label+':primitive dictionary '+String(primitive),()=>source[method](primitive),'TypeError');
        }
      }
    }
    const cursorSource=db.transaction('s').objectStore('s').index('i');
    for(const direction of ['prev','prevunique']) {
      pending.push(new Promise((resolve,reject)=>{
        const found=[],r=cursorSource.openCursor(null,direction);
        r.onerror=()=>reject(r.error);
        r.onsuccess=()=>{const c=r.result;if(c){found.push(c.primaryKey);c.continue()}else{equal('cursor '+direction,found,direction==='prev'?[3,2,1]:[3,1]);resolve()}};
      }));
    }
    await Promise.all(pending);
    for(const method of ['getAll','getAllKeys','getAllRecords']) {
      for(const member of ['count','query']) {
        const tx=db.transaction('s'), s=tx.objectStore('s');
        throws(method+':abort during '+member,()=>s[method]({get [member](){tx.abort();return member==='count'?0:null}}),'TransactionInactiveError');
      }
      const tx=db.transaction('s'),s=tx.objectStore('s');let read=0;
      throws(method+':invalid count stops dictionary',()=>s[method]({count:-1,get direction(){read++;return 'next'},get query(){read++;return 1}}),'TypeError');
      equal(method+':later members not read',read,0);
    }
    const tx=db.transaction(['s','complex']), complete=done(tx);
    const recordsRequest=tx.objectStore('complex').index('i').getAllRecords();
    const multi=request(tx.objectStore('s').index('multi').getAllRecords()).then(rows=>equal('multiEntry record keys',rows.map(r=>[r.key,r.primaryKey]),[['x',1],['x',2],['y',1],['z',3]]));
    const rows=await request(recordsRequest), record=rows[0];
    check('record prototype',Object.getPrototypeOf(record)===IDBRecord.prototype);
    equal('record own properties',Reflect.ownKeys(record),[]);
    equal('record tag',Object.prototype.toString.call(record),'[object IDBRecord]');
    check('value identity and cycle',record.value===record.value && record.value.self===record.value);
    check('different key and primary key',indexedDB.cmp(record.key,record.primaryKey)!==0);
    for(const name of ['key','primaryKey','value']) {
      const desc=Object.getOwnPropertyDescriptor(IDBRecord.prototype,name);
      check('descriptor '+name,typeof desc.get==='function' && desc.set===undefined && desc.enumerable && desc.configurable);
      check('readonly '+name,Reflect.set(record,name,123)===false);
      for(const receiver of [Object.create(IDBRecord.prototype),new Proxy(record,{}),{}]) throws('record brand '+name,()=>desc.get.call(receiver),'TypeError');
    }
    const key=record.key, primaryKey=record.primaryKey;
    key[0].setTime(1000);new Uint8Array(key[1])[0]=99;key[2].push('changed');
    primaryKey[0].setTime(2000);new Uint8Array(primaryKey[1])[0]=99;primaryKey[2].push('changed');
    record.value.payload.name='changed';
    await multi;await complete;
    const again=(await request(db.transaction('complex').objectStore('complex').index('i').getAllRecords()))[0];
    equal('stored index key isolated',[again.key[0].getTime(),Array.from(new Uint8Array(again.key[1])),again.key[2]],[20,[4,5],['\ud800']]);
    equal('stored primary key isolated',[again.primaryKey[0].getTime(),Array.from(new Uint8Array(again.primaryKey[1])),again.primaryKey[2]],[10,[1,2],['\udfff']]);
    equal('stored value isolated',again.value.payload.name,'original');
    const blocker=db.transaction('s','readwrite');blocker.objectStore('s').put({id:4,group:'b',tags:['z']},4);
    const waiting=db.transaction('s'), waitingDone=done(waiting), store=waiting.objectStore('s'), waitingIndex=store.index('i');
    // Both object-store and index requests wait behind the older writer.
    const waitingRequests=[request(store.getAllRecords({direction:'prev',count:2})).then(rows=>equal('queued committed snapshot',rows.map(r=>r.primaryKey),[4,3])), request(store.index('i').getAllRecords({query:'b',direction:'prevunique'})).then(rows=>equal('queued index snapshot',rows.map(r=>r.primaryKey),[3]))];
    const complexTx=db.transaction('complex'), complexSource=complexTx.objectStore('complex');
    const complexKey=[new Date(10),new Uint8Array([1,2]),['\udfff']];let reads=0;
    const queryOptions={get query(){reads++;return complexKey}};
    const snapshot=request(complexSource.getAllRecords(queryOptions)).then(rows=>equal('query captured before mutation',rows.length,1));
    complexKey[0].setTime(999);complexKey[2][0]='changed';equal('query getter once',reads,1);
    await snapshot;await Promise.all(waitingRequests);await waitingDone;
    for(const source of [store,waitingIndex]) {
      for(const method of ['getAll','getAllKeys','getAllRecords']) {
        const order=[];
        throws('inactive '+source.constructor.name+'.'+method,()=>source[method]({get count(){order.push('count');return 1},get direction(){order.push('direction');return 'next'},get query(){order.push('query');return null}}),'TransactionInactiveError');
        equal('inactive conversion '+source.constructor.name+'.'+method,order,['count','direction','query']);
      }
    }
    if(typeof document!=='undefined') {
      const frame=document.createElement('iframe');document.body.appendChild(frame);const child=frame.contentWindow;
      try {
        const rt=db.transaction('complex'), local=rt.objectStore('complex');
        const remote=child.IDBObjectStore.prototype.getAllRecords;
        throws('cross realm dictionary error',()=>remote.call(local,1),'TypeError');
        try {remote.call(local,1)}catch(error){check('TypeError callee realm',error instanceof child.TypeError && !(error instanceof TypeError))}
        const r=remote.call(local);check('request receiver realm',r instanceof IDBRequest && !(r instanceof child.IDBRequest));
        const rr=await request(r);check('result receiver realm',rr instanceof Array && rr[0] instanceof IDBRecord && !(rr[0] instanceof child.IDBRecord));
        for(const name of ['key','primaryKey']) {
          const getter=Object.getOwnPropertyDescriptor(child.IDBRecord.prototype,name).get;
          const value=getter.call(rr[0]);check('key getter callee realm '+name,value instanceof child.Array && value[0] instanceof child.Date && value[1] instanceof child.ArrayBuffer);
          try {getter.call(new Proxy(rr[0],{}));check('getter brand realm '+name,false)}catch(error){check('getter brand realm '+name,error instanceof child.TypeError && !(error instanceof TypeError))}
        }
        const valueGetter=Object.getOwnPropertyDescriptor(child.IDBRecord.prototype,'value').get;
        check('cross realm value identity',valueGetter.call(rr[0])===rr[0].value);
      } finally {frame.remove()}
    }
  } finally {db.close()}
  return {state:checks.every(check=>check.pass)?'pass':'fail',checks};
}
