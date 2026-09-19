globalThis.keyChecks = [];
async function keyModelProbe(name = 'key-model-' + Math.random()) {
  const checks = globalThis.keyChecks;
  const check = (name, run) => {
    try { if (!run()) throw new Error('false'); checks.push({name, pass: true}); }
    catch (error) { checks.push({name, pass: false, detail: String(error)}); }
  };
  const invalid = value => {
    try { IDBKeyRange.only(value); return false; }
    catch (error) { return error.name === 'DataError'; }
  };
  const describe = value => {
    if (typeof value === 'number') return ['number', Object.is(value, -0) ? '0' : String(value)];
    if (typeof value === 'string') return ['string', Array.from({length:value.length}, (_,i)=>value.charCodeAt(i))];
    if (value instanceof Date) return ['date', Date.prototype.getTime.call(value)];
    if (value instanceof ArrayBuffer) return ['binary', Array.from(new Uint8Array(value))];
    if (ArrayBuffer.isView(value)) return ['binary', Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength))];
    if (Array.isArray(value)) return ['array', value.map(describe)];
    throw new Error('unexpected key type');
  };
  const same = (left, right) => JSON.stringify(describe(left)) === JSON.stringify(describe(right));
  const keys = [-Infinity, -1.5, -Number.MIN_VALUE, 0, Number.MIN_VALUE, 1.5,
    Number.MAX_SAFE_INTEGER, 2 ** 53, Number.MAX_VALUE, Infinity,
    new Date(-8640000000000000), new Date(0), new Date(8640000000000000),
    '', '\0', 'a', '\ud800', '\ud800\udc00', '\udbff', '\udc00', '\ufffd', '\uffff',
    new Uint8Array([]), new Uint8Array([0]), new Uint8Array([0,255]), new Uint8Array([1]),
    [], [0], [0, new Date(0)], [[0]]];
  keys.forEach((key, i) => {
    check('key ' + i + ': compare reflexive', () => indexedDB.cmp(key,key) === 0);
    check('key ' + i + ': range roundtrip', () => same(IDBKeyRange.only(key).lower, key));
    if (i) check('key ' + i + ': cross-type and payload order', () => indexedDB.cmp(keys[i-1],key) === -1 && indexedDB.cmp(key,keys[i-1]) === 1);
  });
  check('negative zero range value is preserved', () => Object.is(IDBKeyRange.only(-0).lower, -0));
  check('signed zeros compare equally', () => indexedDB.cmp(-0,0) === 0 && indexedDB.cmp(0,-0) === 0);
  const date = new Date(25);
  date.valueOf = date.toString = () => { throw new Error('coercion'); };
  check('Date uses internal value', () => same(IDBKeyRange.only(date).lower, new Date(25)));
  let coercions = 0;
  for (const [label, value] of [['Number',new Number(3)], ['String',new String('a')]]) {
    value.valueOf = value.toString = () => { coercions++; return 3; };
    check('boxed ' + label + ' is invalid', () => invalid(value));
  }
  check('boxed keys do not coerce', () => coercions === 0);
  const cycle = []; cycle[0] = cycle;
  const repeated = [];
  for (const [label, key] of [['NaN',NaN],['invalid Date',new Date(NaN)],['null',null],['bigint',1n],
    ['undefined entry',[undefined]],['cycle',cycle],
    ['array Proxy',new Proxy([1], {})],['Date Proxy',new Proxy(new Date(1), {})],['binary Proxy',new Proxy(new Uint8Array([1]), {})]]) {
    check('invalid ' + label, () => invalid(key));
  }
  check('repeated array is not a cycle', () => same(IDBKeyRange.only([repeated,repeated]).lower, [[],[]]));
  let inheritedReads = 0;
  const holes = Array(1);
  Object.setPrototypeOf(holes, Object.defineProperty({}, '0', {get(){ inheritedReads++; return 1; }}));
  check('array holes ignore inherited getters', () => invalid(holes) && inheritedReads === 0);
  let nested = 1;
  for (let i = 0; i < 100; i++) nested = [nested];
  check('deep array key is valid', () => indexedDB.cmp(nested, nested) === 0);
  const allBytes = new Uint8Array([9,0,128,255,7]);
  check('typed array honors offset and length', () => same(IDBKeyRange.only(allBytes.subarray(1,4)).lower, new Uint8Array([0,128,255])));
  check('DataView honors offset and length', () => same(IDBKeyRange.only(new DataView(allBytes.buffer,2,2)).lower,new Uint8Array([128,255])));
  const range = IDBKeyRange.only(allBytes.subarray(1,4));
  allBytes.fill(4);
  check('binary key snapshots input', () => same(range.lower,new Uint8Array([0,128,255])));
  const rangeDate = new Date(13), dateRange = IDBKeyRange.only(rangeDate);
  rangeDate.setTime(200);
  check('Date key snapshots input', () => same(dateRange.lower,new Date(13)));
  const detached = new ArrayBuffer(3), detachedView = new Uint8Array(detached), detachedDataView = new DataView(detached);
  structuredClone(detached,{transfer:[detached]});
  for (const [label,value] of [['ArrayBuffer',detached],['TypedArray',detachedView],['DataView',detachedDataView]]) check('detached '+label,()=>invalid(value));
  const namedError = (fn, name) => { try { fn(); return false; } catch(error) { return error.name === name; } };
  const resizable = new ArrayBuffer(4,{maxByteLength:8});
  const resizableValues = [resizable,new Uint8Array(resizable),new DataView(resizable),new Uint8Array(resizable,4,0)];
  const oobBuffer = new ArrayBuffer(4,{maxByteLength:8}), oobView = new Uint8Array(oobBuffer,2,2);
  oobBuffer.resize(1); resizableValues.push(oobView);
  resizableValues.forEach((value,i)=>{
    check('resizable '+i+': range TypeError',()=>namedError(()=>IDBKeyRange.only(value),'TypeError'));
    check('resizable '+i+': compare TypeError',()=>namedError(()=>indexedDB.cmp(value,0),'TypeError'));
  });
  const staticConsumers = [
    key=>indexedDB.cmp(key,0), key=>indexedDB.cmp(0,key), key=>IDBKeyRange.only(key),
    key=>IDBKeyRange.lowerBound(key), key=>IDBKeyRange.upperBound(key),
    key=>IDBKeyRange.bound(key,0), key=>IDBKeyRange.bound(0,key), key=>IDBKeyRange.only(0).includes(key)
  ];
  for(const [i,error] of [undefined,17,{sentinel:true},new TypeError('original')].entries()) {
    staticConsumers.forEach((consume,j)=>{
      check('static conversion '+j+': exact exception '+i,()=>{
        const key = Object.defineProperty([],0,{get(){throw error;}});
        try {consume(key);return false;}catch(caught){return caught === error;}
      });
    });
  }
  const request = req => new Promise((resolve,reject) => { req.onsuccess=()=>resolve(req.result); req.onerror=()=>reject(req.error); });
  const done = tx => new Promise((resolve,reject) => { tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error || new Error('transaction aborted')); });
  const open = indexedDB.open(name,1);
  open.onupgradeneeded = () => {
    const store = open.result.createObjectStore('keys');
    store.createIndex('unique','key',{unique:true});
    open.result.createObjectStore('generator',{autoIncrement:true});
  };
  let db = await request(open);
  try {
    const validationTx=db.transaction('keys','readwrite'), validationDone=done(validationTx), validationStore=validationTx.objectStore('keys');
    const validationIndex=validationStore.index('unique');
    const consumers = [
      key=>validationStore.put('v',key), key=>validationStore.add('v',key), key=>validationStore.delete(key),
      ...[validationStore,validationIndex].flatMap(source=>['get','getKey','getAll','getAllKeys','count','openCursor','openKeyCursor'].map(method=>key=>source[method](key)))
    ];
    consumers.forEach((consume,i)=>{
      check('operation '+i+': buffer TypeError',()=>namedError(()=>consume(resizable),'TypeError'));
      const sentinel={operation:i};
      const throwing=Object.defineProperty([],0,{get(){throw sentinel;}});
      check('operation '+i+': exact getter exception',()=>{try{consume(throwing);return false;}catch(error){return error===sentinel;}});
    });
    await validationDone;
    let tx = db.transaction('keys','readwrite'), completion = done(tx), store = tx.objectStore('keys');
    for (let i=0;i<keys.length;i++) {
      const actual = await request(store.put({key:keys[i], index:i},keys[i]));
      check('stored key '+i+': returned type',()=>same(actual,keys[i]));
    }
    await completion;
    db.close();
    db = await request(indexedDB.open(name));
    tx = db.transaction('keys'); completion=done(tx);store=tx.objectStore('keys');
    const readKeys = await request(store.getAllKeys());
    check('reopen: all keys distinct and ordered',()=>readKeys.length === keys.length && readKeys.every((key,i)=>same(key,keys[i])));
    const indexedKeys = await request(store.index('unique').getAllKeys());
    check('index: all key types distinct and ordered',()=>indexedKeys.length === keys.length && indexedKeys.every((key,i)=>same(key,keys[i])));
    for (let i=0;i<keys.length;i++) {
      const value = await request(store.get(keys[i]));
      check('reopen: key '+i+' retrieves own record',()=>value.index===i && same(value.key,keys[i]));
    }
    const numberOptions=Object.assign(new Number(9),{query:1.5});
    const stringOptions=Object.assign(new String('a'),{query:new Date(0)});
    for (const [label,options,expected] of [['Number',numberOptions,keys.indexOf(1.5)],['String',stringOptions,11]]) {
      options.valueOf=()=>{throw new Error('options coercion');};
      const values=await request(store.getAll(options));
      check('boxed '+label+' is getAll dictionary',()=>values.length===1 && values[0].index===expected);
    }
    await completion;
    tx=db.transaction('generator','readwrite'); completion=done(tx);store=tx.objectStore('generator');
    await request(store.put('date',new Date(1000)));
    const first=await request(store.put('first'));
    check('generator starts at one after Date',()=>first===1);
    await request(store.put('fraction',3.9));
    const fourth=await request(store.put('fourth'));
    check('generator advances by numeric floor',()=>fourth===4);
    await request(store.put('last explicit',Number.MAX_SAFE_INTEGER));
    const last=await request(store.put('last generated'));
    check('generator includes 2^53',()=>last===2**53);
    const overflow=store.put('overflow');
    const overflowName=await new Promise(resolve=>{overflow.onerror=event=>{event.preventDefault();resolve(overflow.error.name);};overflow.onsuccess=()=>resolve('success');});
    check('generator fails after 2^53',()=>overflowName==='ConstraintError');
    const infinite=await request(store.put('infinity',Infinity));
    check('explicit Infinity survives exhausted generator',()=>infinite===Infinity);
    await completion;
  } finally { db.close(); }
  await request(indexedDB.deleteDatabase(name));
  return {state:checks.every(item=>item.pass)?'pass':'fail',checks};
}
