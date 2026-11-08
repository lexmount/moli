globalThis.cursorChecks = [];
async function cursorStateProbe(name = 'cursor-state-' + Math.random()) {
  const checks = globalThis.cursorChecks;
  const check = (label, pass, actual = '') => checks.push({label,pass:!!pass,actual:JSON.stringify(actual)});
  const equal = (label, actual, expected) => check(label,JSON.stringify(actual)===JSON.stringify(expected),actual);
  const throws = (label, call, name) => {
    try {call();check(label,false,'accepted');}
    catch(error) {check(label,error.name===name,error.name);}
  };
  const request = r => new Promise((resolve,reject) => {r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
  const done = tx => new Promise((resolve,reject) => {tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error);});
  const attributes = ['source','request','direction','key','primaryKey','value'];
  const methods = Object.fromEntries(['advance','continue','continuePrimaryKey','update','delete'].map(name => [name,IDBCursor.prototype[name]]));
  const getters = Object.fromEntries(attributes.map(name => [name,Object.getOwnPropertyDescriptor(name==='value'?IDBCursorWithValue.prototype:IDBCursor.prototype,name)?.get]));
  const read = (cursor,name) => getters[name] ? getters[name].call(cursor) : cursor[name];
  for(const attribute of attributes) {
    const descriptor = Object.getOwnPropertyDescriptor(attribute==='value'?IDBCursorWithValue.prototype:IDBCursor.prototype,attribute);
    check('readonly prototype descriptor '+attribute,descriptor && typeof descriptor.get==='function' && descriptor.set===undefined && descriptor.enumerable && descriptor.configurable && descriptor.get.length===0);
  }
  for(const [method,length] of [['advance',1],['continue',0],['continuePrimaryKey',2],['update',1],['delete',0]]) equal(method+' arity',methods[method].length,length);
  const kinds = [
    ['array', n => [[n]], (key,n) => key[0][0]=n],
    ['date', n => new Date(n), (key,n) => key.setTime(n)],
    ['binary', n => new Uint8Array([n]).buffer, (key,n) => new Uint8Array(key)[0]=n]
  ];
  const opening = indexedDB.open(name,1);
  opening.onupgradeneeded = () => {
    for(const [kind,make] of kinds) {
      const store=opening.result.createObjectStore(kind);store.createIndex('i','index');
      for(let id=1;id<=3;id++) {const value={id,index:make(id),nested:{tag:id}};value.self=value;store.put(value,make(id));}
    }
    const duplicate=opening.result.createObjectStore('duplicate');duplicate.createIndex('i','index');
    duplicate.put({id:1,index:[1]},[1]);duplicate.put({id:2,index:[1]},[2]);duplicate.put({id:3,index:[2]},[3]);
    opening.result.createObjectStore('undefined').put(undefined,1);
  };
  const db=await request(opening);
  let publicReads=0;
  const poison = (cursor,hasValue) => {
    if(!attributes.every(name=>getters[name])) return;
    for(const name of attributes) {
      if(name==='value' && !hasValue) continue;
      Object.defineProperty(cursor,name,{get(){publicReads++;throw new Error('public cursor attribute read');}});
    }
    for(const name of ['__moli_idb_cursor_source','__moli_idb_cursor_request','__moli_idb_cursor_key_cache','__moli_idb_cursor_primary_key_cache','__moli_idb_cursor_value_cache','__moliIndexedDbCursorEntries','__moliIndexedDbCursorPosition']) Object.defineProperty(cursor,name,{value:null});
    Object.setPrototypeOf(cursor,null);Object.freeze(cursor);
  };
  try {
    for(const [kind,make,mutate] of kinds) for(const sourceName of ['store','index']) for(const method of ['openCursor','openKeyCursor']) for(const direction of ['next','prev']) {
      const label=[kind,sourceName,method,direction].join(' '), hasValue=method==='openCursor';
      const tx=db.transaction(kind),complete=done(tx),store=tx.objectStore(kind),source=sourceName==='store'?store:store.index('i');
      const r=source[method](null,direction),cursor=await request(r),first=direction==='next'?1:3;
      equal(label+' own properties',Object.keys(cursor),[]);
      check(label+' source and request',read(cursor,'source')===source && read(cursor,'request')===r);
      equal(label+' direction',read(cursor,'direction'),direction);
      check(label+' interface',hasValue ? cursor instanceof IDBCursorWithValue : cursor instanceof IDBCursor && !(cursor instanceof IDBCursorWithValue));
      if(!hasValue) {
        check(label+' no value attribute',!('value' in cursor));
        if(getters.value) throws(label+' rejects value getter',()=>getters.value.call(cursor),'TypeError');
      }
      const key=read(cursor,'key'),primaryKey=read(cursor,'primaryKey'),value=hasValue?read(cursor,'value'):undefined;
      check(label+' independent key caches',key!==primaryKey && read(cursor,'key')===key && read(cursor,'primaryKey')===primaryKey);
      for(const attribute of attributes) {
        if(attribute==='value' && !hasValue) continue;
        check(label+' readonly '+attribute,Reflect.set(cursor,attribute,read(cursor,attribute))===false);
      }
      mutate(key,9);mutate(primaryKey,9);
      if(hasValue) {value.nested.tag=99;value.cursor=cursor;check(label+' cyclic cached value',value.self===value && read(cursor,'value')===value);}
      poison(cursor,hasValue);
      try {methods.continue.call(cursor,make(2));check(label+' continue ignores exposed keys',true);}
      catch(error) {check(label+' continue ignores exposed keys',false,error.name);mutate(key,first);mutate(primaryKey,first);methods.continue.call(cursor,make(2));}
      check(label+' retains pending cache',read(cursor,'key')===key && read(cursor,'primaryKey')===primaryKey && (!hasValue || read(cursor,'value')===value));
      check(label+' same cursor',await request(r)===cursor);
      const nextKey=read(cursor,'key'),nextPrimary=read(cursor,'primaryKey'),nextValue=hasValue?read(cursor,'value'):undefined;
      check(label+' fresh next keys',nextKey!==key && nextPrimary!==primaryKey && indexedDB.cmp(nextKey,make(2))===0 && indexedDB.cmp(nextPrimary,make(2))===0);
      if(hasValue) check(label+' fresh next value',nextValue!==value && nextValue.id===2 && nextValue.nested.tag===2 && nextValue.self===nextValue);
      methods.advance.call(cursor,100);
      check(label+' retains pending exhaustion',read(cursor,'key')===nextKey && read(cursor,'primaryKey')===nextPrimary);
      check(label+' exhausted request is null',await request(r)===null);
      await complete;
      // Cursor iteration clears key/value when no record is found. Chromium
      // currently retains already materialized values; keep that difference
      // visible in the browser comparison rather than treating it as required.
      check(label+' clears exhausted cache',read(cursor,'key')===undefined && read(cursor,'primaryKey')===undefined && (!hasValue || read(cursor,'value')===undefined));
      check(label+' metadata after completion',read(cursor,'source')===source && read(cursor,'request')===r && read(cursor,'direction')===direction);
    }
    // Receiver checks precede conversion and apply to both cursor interfaces.
    for(const method of ['openCursor','openKeyCursor']) {
      const cursor=await request(db.transaction('array').objectStore('array')[method]());
      const revoked=Proxy.revocable(cursor,{});revoked.revoke();
      for(const [label,receiver] of [['plain',{}],['forged',Object.create(Object.getPrototypeOf(cursor))],['inheritor',Object.create(cursor)],['proxy',new Proxy(cursor,{})],['revoked',revoked.proxy],['null',null]]) {
        let reads=0;
        const number={valueOf(){reads++;return 1}}, key=Object.defineProperty([],0,{get(){reads++;return 1}}), value={get id(){reads++;return 1}};
        for(const [name,args] of [['advance',[number]],['continue',[key]],['continuePrimaryKey',[key,key]],['update',[value]],['delete',[]]]) throws(method+' '+name+' brand '+label,()=>methods[name].apply(receiver,args),'TypeError');
        equal(method+' brand before conversion '+label,reads,0);
        for(const name of attributes) if(getters[name]) throws(method+' getter '+name+' brand '+label,()=>getters[name].call(receiver),'TypeError');
      }
    }
    // Mutation uses the native primary key even after both exposed keys change.
    for(const sourceName of ['store','index']) {
      const tx=db.transaction('array','readwrite'),complete=done(tx),store=tx.objectStore('array'),source=sourceName==='store'?store:store.index('i');
      const cursor=await request(source.openCursor());
      const key=read(cursor,'key'),primary=read(cursor,'primaryKey'),value=read(cursor,'value');key[0][0]=9;primary[0][0]=9;
      poison(cursor,true);
      const updated=await request(methods.update.call(cursor,{id:10,index:[[1]],nested:{tag:10}}));
      equal(sourceName+' update native primary key',updated,[[1]]);
      check(sourceName+' update preserves cached value',read(cursor,'value')===value);
      equal(sourceName+' update persisted',(await request(store.get([[1]]))).id,10);
      await request(methods.delete.call(cursor));
      equal(sourceName+' delete native primary key',await request(store.count([[1]])),0);
      // Restore for the next variant and preserve the cursor's old cached value.
      store.put({id:1,index:[[1]],nested:{tag:1}},[[1]]);
      await complete;
      check(sourceName+' mutation leaves cached key',read(cursor,'key')===key && read(cursor,'primaryKey')===primary && read(cursor,'value')===value);
    }
    {
      const tx=db.transaction('duplicate'),complete=done(tx),source=tx.objectStore('duplicate').index('i'),r=source.openCursor();
      const cursor=await request(r),key=read(cursor,'key'),primary=read(cursor,'primaryKey');key[0]=9;primary[0]=9;poison(cursor,true);
      try {methods.continuePrimaryKey.call(cursor,[1],[2]);check('continuePrimaryKey ignores exposed tuple',true);}
      catch(error) {check('continuePrimaryKey ignores exposed tuple',false,error.name);key[0]=1;primary[0]=1;methods.continuePrimaryKey.call(cursor,[1],[2]);}
      await request(r);
      equal('continuePrimaryKey next tuple',[read(cursor,'key'),read(cursor,'primaryKey')],[[1],[2]]);
      check('duplicate index key refreshes cache',read(cursor,'key')!==key);
      await complete;
    }
    const undefinedCursor=await request(db.transaction('undefined').objectStore('undefined').openCursor());
    equal('undefined value is readable',read(undefinedCursor,'value'),undefined);
    equal('undefined value stays readable',read(undefinedCursor,'value'),undefined);
    equal('operations never read public cursor properties',publicReads,0);
    for(const sourceName of ['store','index']) for(const method of ['openCursor','openKeyCursor']) {
      const tx=db.transaction('array'),complete=done(tx),store=tx.objectStore('array'),source=sourceName==='store'?store:store.index('i'),r=source[method](),cursor=await request(r);
      methods.advance.call(cursor,2);await request(r);methods.continue.call(cursor);await request(r);await complete;
      for(const name of method==='openCursor'?['key','primaryKey','value']:['key','primaryKey']) equal(sourceName+' '+method+' unread exhausted '+name,read(cursor,name),undefined);
    }
    if(typeof document!=='undefined') {
      const frame=document.createElement('iframe');document.body.appendChild(frame);const child=frame.contentWindow;
      try {
        for(const first of ['child','parent']) {
          const tx=db.transaction('array'),complete=done(tx),r=tx.objectStore('array').openCursor(),cursor=await request(r);
          for(const name of ['key','primaryKey','value']) {
            const descriptor=Object.getOwnPropertyDescriptor(name==='value'?child.IDBCursorWithValue.prototype:child.IDBCursor.prototype,name);
            if(!descriptor || !descriptor.get) {check('foreign getter '+name,false);continue;}
            const getter=descriptor.get,initial=first==='child'?getter.call(cursor):read(cursor,name);
            check(first+' first '+name+' realm',first==='child'?initial instanceof child.Object && !(initial instanceof Object):initial instanceof Object && !(initial instanceof child.Object));
            check(first+' '+name+' cross realm identity',getter.call(cursor)===initial && read(cursor,name)===initial);
            try {getter.call(new Proxy(cursor,{}));check(name+' getter error realm',false);}
            catch(error) {check(name+' getter error realm',error instanceof child.TypeError && !(error instanceof TypeError));}
          }
          for(const [method,args] of [['advance',[1]],['continue',[]],['continuePrimaryKey',[[1],[1]]],['update',[{}]],['delete',[]]]) {
            try {child.IDBCursor.prototype[method].apply(new Proxy(cursor,{}),args);check(method+' error realm',false);}
            catch(error) {check(method+' error realm',error instanceof child.TypeError && !(error instanceof TypeError));}
          }
          child.IDBCursor.prototype.continue.call(cursor);
          await request(r);
          const nextGetter=Object.getOwnPropertyDescriptor(child.IDBCursor.prototype,'key')?.get;
          if(nextGetter) check(first+' new position chooses realm again',nextGetter.call(cursor) instanceof child.Array && read(cursor,'key') instanceof child.Array);
          await complete;
        }
        // No getter is read at the final position until after exhaustion.
        const tx=db.transaction('array'),complete=done(tx),r=tx.objectStore('array').openCursor(),cursor=await request(r);
        methods.advance.call(cursor,2);await request(r);methods.continue.call(cursor);await request(r);await complete;
        for(const name of ['key','primaryKey','value']) {
          const getter=Object.getOwnPropertyDescriptor(name==='value'?child.IDBCursorWithValue.prototype:child.IDBCursor.prototype,name)?.get;
          if(getter) {equal('lazy exhausted '+name,getter.call(cursor),undefined);equal('lazy exhausted '+name+' parent',read(cursor,name),undefined);}
        }
      } finally {frame.remove();}
    }
  } finally {db.close();}
  await request(indexedDB.deleteDatabase(name));
  return {state:checks.every(check=>check.pass)?'pass':'fail',checks};
}
