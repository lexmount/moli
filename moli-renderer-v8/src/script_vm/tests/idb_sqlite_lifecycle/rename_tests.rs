use super::*;

#[tokio::test]
async fn idb_sqlite_rename_preserves_canonical_handles_generator_and_cursor_fifo() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.renameEvents=[];
        const store=upgrade.objectStore('records');
        const index=store.index('unique');
        for(const group of ['a','b','c','d']) store.put({group});
        store.name='\ud800';
        index.name='';
        globalThis.canonicalRename=upgrade.objectStore('\ud800')===store && store.index('')===index;
        globalThis.reusedStore=db.createObjectStore('records');
        globalThis.reusedIndex=store.createIndex('unique','group');
        canonicalRename &&= reusedStore!==store && reusedIndex!==index;
        const request=index.openCursor();
        request.onerror=()=>renameEvents.push('error:'+request.error.name);
        request.onsuccess=()=>{
          const cursor=request.result;
          if(!cursor) {renameEvents.push('end');return;}
          renameEvents.push(cursor.key+':'+cursor.primaryKey);
          if(cursor.key==='a') {
            cursor.continue();
            // The admitted cursor command must keep the pre-rename names.
            store.name='renamed'; index.name='renamed-index';
          } else if(cursor.key==='b') {
            cursor.advance(1);
          } else if(cursor.key==='c') {
            cursor.continuePrimaryKey('d',4);
          } else {
            cursor.continue();
            store.name='final'; index.name='final-index';
          }
        };
        upgrade.oncomplete=()=>renameEvents.push('complete');
        upgrade.onabort=()=>renameEvents.push('abort:'+upgrade.error?.name);
        'queued'
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("String(canonicalRename)").unwrap(), "true");
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(renameEvents)").unwrap(),
        r#"["a:1","b:2","c:3","d:4","end","complete"]"#
    );
    let metadata = backend.client.committed().unwrap();
    assert_eq!(metadata.stores.len(), 2);
    assert!(metadata.stores.contains_key(&"records".into()));
    let store = &metadata.stores[&moli_indexeddb::IndexedDbName::from("final")];
    assert!(store.indexes.contains_key(&"final-index".into()));
    assert!(store.indexes.contains_key(&"unique".into()));
    vm.eval(r#"
        globalThis.afterRename=[];
        const tx=db.transaction('final','readwrite');
        const current=tx.objectStore('final');
        current.index('final-index').getAll().onsuccess=e=>afterRename.push(e.target.result.map(v=>v.group).join(''));
        current.put({group:'e'}).onsuccess=e=>afterRename.push(e.target.result);
        tx.oncomplete=()=>afterRename.push('complete'); 'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(afterRename)").unwrap(),
        r#"["abcd",5,"complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_rename_abort_restores_existing_identity_and_keeps_created_names() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.store=upgrade.objectStore('records');
        globalThis.index=store.index('unique');
        store.name='moved'; index.name='moved-index';
        globalThis.createdIndex=store.createIndex('unique','group');
        createdIndex.name='created-index';
        globalThis.createdStore=db.createObjectStore('records');
        createdStore.name='created-store';
        store.put({group:'discarded'}).onerror=e=>e.preventDefault();
        upgrade.abort();
        globalThis.rollbackNow=[store.name,index.name,createdStore.name,createdIndex.name,
          Array.from(db.objectStoreNames).join(','),Array.from(upgrade.objectStoreNames).join(','),
          Array.from(store.indexNames).join(','),createdStore.indexNames.length];
        globalThis.deletedErrors=[];
        for(const handle of [createdStore,createdIndex]) {
          try {handle.name='forbidden';} catch(error) {deletedErrors.push(error.name);}
        } 'aborting'
    "#,
    )
    .unwrap();
    let expected =
        r#"["records","unique","created-store","created-index","records","records","unique",0]"#;
    assert_eq!(vm.eval("JSON.stringify(rollbackNow)").unwrap(), expected);
    assert_eq!(
        vm.eval("JSON.stringify(deletedErrors)").unwrap(),
        r#"["InvalidStateError","InvalidStateError"]"#
    );
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("JSON.stringify([store.name,index.name,createdStore.name,createdIndex.name,Array.from(db.objectStoreNames).join(','),Array.from(upgrade.objectStoreNames).join(','),Array.from(store.indexNames).join(','),createdStore.indexNames.length])").unwrap(), expected);
    let metadata = backend.client.committed().unwrap();
    assert_eq!(metadata.version, 1);
    assert_eq!(metadata.stores.len(), 1);
    assert_eq!(
        metadata.stores[&moli_indexeddb::IndexedDbName::from("records")]
            .indexes
            .len(),
        1
    );
    vm.eval("globalThis.saved=-1;db.transaction('records').objectStore('records').count().onsuccess=e=>saved=e.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(saved)").unwrap(), "0");
}

pub(super) fn registered_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let target = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
    let handler = crate::util::new_null_prototype_object(scope);
    let proxy = v8::Proxy::new(scope, target, handler).unwrap();
    moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
    rv.set(proxy.into());
}

#[tokio::test]
async fn idb_sqlite_rename_uses_native_identity_across_realms_shadows_and_conversion_reentry() {
    let (mut vm, loader, backend) = fixture();
    vm.with_default_context_scope(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let function = v8::Function::new(scope, registered_proxy).unwrap();
        global
            .set(
                scope,
                crate::util::v8str(scope, "registeredProxy").into(),
                function.into(),
            )
            .unwrap();
        Ok(())
    })
    .unwrap();
    backend.upgrade(&mut vm);
    vm.eval(r#"
        document.body.innerHTML='<iframe></iframe>';
        const other=document.querySelector('iframe').contentWindow;
        const store=upgrade.objectStore('records');const index=store.index('unique');
        const nativeStore=registeredProxy(store);const nativeIndex=registeredProxy(index);
        const s=Object.getOwnPropertyDescriptor(other.IDBObjectStore.prototype,'name');
        const i=Object.getOwnPropertyDescriptor(other.IDBIndex.prototype,'name');
        globalThis.nativeNameResults=[!Object.hasOwn(store,'name'),!Object.hasOwn(index,'name')];
        globalThis.nameShadowCalls=0;
        for(const handle of [store,index]) Object.defineProperty(handle,'name',{
          configurable:true,get(){nameShadowCalls++;throw 42},set(){nameShadowCalls++;throw 42}});
        s.set.call(nativeStore,{toString(){s.set.call(nativeStore,'middle');return '\ud800'}});
        i.set.call(nativeIndex,{toString(){i.set.call(nativeIndex,'');return '\udc00'}});
        nativeNameResults.push(s.get.call(nativeStore).charCodeAt(0),i.get.call(nativeIndex).charCodeAt(0),
          upgrade.objectStore('\ud800')===store,store.index('\udc00')===index);
        globalThis.nativeCount=-1;
        index.count().onsuccess=e=>nativeCount=e.target.result;
        'queued'
    "#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(nativeNameResults)").unwrap(),
        "[true,true,55296,56320,true,true]"
    );
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("[nameShadowCalls,nativeCount].join(',')").unwrap(),
        "0,0"
    );
    let metadata = backend.client.committed().unwrap();
    let store = &metadata.stores[&moli_indexeddb::IndexedDbName::from_utf16(vec![0xd800])];
    assert!(
        store
            .indexes
            .contains_key(&moli_indexeddb::IndexedDbName::from_utf16(vec![0xdc00]))
    );
}
