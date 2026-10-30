use super::*;

const PROBE: &str = include_str!("indexed_db_cursor_gc.js");

#[test]
fn indexed_db_completed_cursor_graphs_collect_and_retained_cursors_survive_gc() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-cursor-gc.test/");
    vm.eval(&format!("{PROBE}\nprepareIndexedDbCursorGc().then(value => globalThis.cursorGcReady=value, error => globalThis.cursorGcReady=String(error)); 'scheduled'"))
        .unwrap();
    // There are no author references to the active open request. The selected
    // task itself must retain it until its events and Promise reactions finish.
    let collect = |vm: &mut crate::runtime::PageVmTaskExecutorTestHarness| {
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
    };
    collect(&mut vm);
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(cursorGcReady)")
            .unwrap(),
        "prepared"
    );
    collect(&mut vm);
    assert_eq!(vm.eval("checkIndexedDbCursorGc()").unwrap(), "true");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert_eq!(
            crate::context_bootstrap::indexed_db_native_cursor_count_for_test(scope),
            1
        );
        Ok(())
    })
    .unwrap();
    vm.eval("delete globalThis.cursorGcHeld").unwrap();
    collect(&mut vm);
    assert_eq!(vm.eval("checkReleasedIndexedDbCursorGc()").unwrap(), "true");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert_eq!(
            crate::context_bootstrap::indexed_db_native_cursor_count_for_test(scope),
            0
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn indexed_db_schema_key_path_backreferences_do_not_root_discarded_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-schema-gc.test/");
    vm.eval(
        r#"globalThis.schemaRefs=[];
      const request=indexedDB.open('schema-gc',1);
      request.onupgradeneeded=()=>{
        const store=request.result.createObjectStore('items',{keyPath:['a','b']});
        const index=store.createIndex('by-value',['x','y']);
        store.keyPath.owner=store; index.keyPath.owner=index;
        schemaRefs.push(new WeakRef(store),new WeakRef(index));
      };
      request.onsuccess=()=>{request.result.close();globalThis.schemaGcReady=true;};
      'scheduled'"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(schemaGcReady)")
            .unwrap(),
        "true"
    );
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        vm.eval("schemaRefs.length===2 && schemaRefs.every(ref=>ref.deref()===undefined)")
            .unwrap(),
        "true"
    );
}

#[test]
fn indexed_db_schema_getters_accept_registered_native_proxies_in_callee_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-schema-proxy.test/");
    vm.eval(
        r#"document.body.innerHTML='<iframe></iframe>';
      globalThis.schemaChild=document.querySelector('iframe').contentWindow;
      const request=indexedDB.open('schema-proxy',1);
      request.onupgradeneeded=()=>{
        const store=request.result.createObjectStore('s',{keyPath:['id']});
        const index=store.createIndex('i',['value']);
        globalThis.schemaValues=[store,index,request.transaction];
      };
      request.onsuccess=()=>globalThis.schemaDatabase=request.result;"#,
    )
    .unwrap();
    vm.eval_after_selected_page_tasks("'prepared'").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "schemaValues");
        let values =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let proxies = v8::Array::new(scope, 3);
        for index in 0..3 {
            let target =
                v8::Local::<v8::Object>::try_from(values.get_index(scope, index).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(proxies.set_index(scope, index, proxy.into()), Some(true));
        }
        let key = crate::util::v8str(scope, "schemaProxies");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxies.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(()=>{
      const interfaces=['IDBObjectStore','IDBIndex','IDBTransaction'];
      const fields=[['keyPath','autoIncrement','indexNames','transaction'],['keyPath','multiEntry','unique','objectStore'],['objectStoreNames']];
      for(const realm of [window,schemaChild]) for(let i=0;i<3;i++) for(const name of fields[i]) {
        const getter=Object.getOwnPropertyDescriptor(realm[interfaces[i]].prototype,name).get;
        const expected=getter.call(schemaValues[i]),actual=getter.call(schemaProxies[i]);
        if(name==='indexNames' || name==='objectStoreNames') {
          if(Object.getPrototypeOf(actual)!==realm.DOMStringList.prototype ||
             JSON.stringify(Array.from(actual))!==JSON.stringify(Array.from(expected))) throw Error('native list realm');
        } else if(actual!==expected) throw Error('native schema identity');
        let traps=0;
        const author=new Proxy(schemaProxies[i],{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 43;}});
        const revoked=Proxy.revocable(schemaProxies[i],{});revoked.revoke();
        for(const receiver of [author,revoked.proxy,Object.create(schemaValues[i])]) {
          let error;try {getter.call(receiver);} catch(caught) {error=caught;}
          if(Object.getPrototypeOf(error)!==realm.TypeError.prototype || traps) throw Error('author schema receiver');
        }
      }
      schemaDatabase.close();return true;
    })()"#).unwrap(), "true");
}
