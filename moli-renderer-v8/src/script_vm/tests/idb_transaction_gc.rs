use super::*;
use crate::runtime::PageVmTaskExecutorTestHarness;

fn collect(vm: &mut PageVmTaskExecutorTestHarness) {
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
}

fn native_entries(vm: &mut PageVmTaskExecutorTestHarness) -> usize {
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        Ok(crate::context_bootstrap::indexed_db_transaction_state_entries_for_test(scope, global))
    })
    .unwrap()
}

#[test]
fn indexed_db_discarded_transactions_release_scope_state_while_retained_getters_survive_gc() {
    let mut vm = new_storage_page_task_executor_test_vm("https://idb-transaction-gc.test/");
    vm.eval(
        r#"globalThis.txGcRefs=[];
        const open=indexedDB.open('transaction-gc',1);
        open.onupgradeneeded=()=>{
          txGcRefs.push(new WeakRef(open.transaction));
          for(const name of ['', 'kept', '\ud800']) open.result.createObjectStore(name);
        };
        open.onsuccess=()=>{
          const db=open.result;
          let remaining=16;
          for(let index=0;index<16;index++) {
            const tx=db.transaction(['kept','\ud800','']);
            txGcRefs.push(new WeakRef(tx));
            if(index===0) globalThis.txGcHeld=tx;
            const finish=()=>{
              if(--remaining===0) {db.close();globalThis.txGcReady=true;}
            };
            tx.oncomplete=finish;
            tx.onabort=finish;
            if(index%2) tx.abort();
          }
        }; 'scheduled'"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(txGcReady)")
            .unwrap(),
        "true"
    );

    collect(&mut vm);
    assert_eq!(
        vm.eval("String(txGcRefs.filter(ref=>ref.deref()!==undefined).length)")
            .unwrap(),
        "1"
    );
    assert_eq!(native_entries(&mut vm), 2);
    assert_eq!(
        vm.eval(
            r#"txGcHeld.mode==='readonly' && txGcHeld.error===null &&
            txGcHeld.objectStoreNames.length===3 &&
            txGcHeld.objectStoreNames[0]==='' &&
            txGcHeld.objectStoreNames[1]==='kept' &&
            txGcHeld.objectStoreNames[2].charCodeAt(0)===0xd800"#
        )
        .unwrap(),
        "true"
    );

    vm.eval("globalThis.txGcSnapshot=txGcHeld.objectStoreNames; delete globalThis.txGcHeld")
        .unwrap();
    collect(&mut vm);
    assert_eq!(
        vm.eval("txGcRefs.length===17 && txGcRefs.every(ref=>ref.deref()===undefined)")
            .unwrap(),
        "true"
    );
    assert_eq!(native_entries(&mut vm), 0);
    assert_eq!(
        vm.eval("txGcSnapshot.length===3 && txGcSnapshot[0]==='' && txGcSnapshot[1]==='kept' && txGcSnapshot[2].charCodeAt(0)===0xd800")
            .unwrap(),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn indexed_db_queued_transactions_keep_native_ownership_until_sql_completion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-transaction-active-gc.test/",
        &loader,
    );
    vm.eval(
        r#"globalThis.txGcRefs=[];
        globalThis.txGcEvents=[];
        globalThis.txGcErrors=[];
        const open=indexedDB.open('active-gc',1);
        open.onupgradeneeded=()=>open.result.createObjectStore('records');
        open.onsuccess=()=>{globalThis.txGcDatabase=open.result;}; 'scheduled'"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(!!globalThis.txGcDatabase)",
        "true",
        "transaction GC database setup",
    )
    .await;
    vm.eval(
        r#"for(let index=0;index<4;index++) {
          const tx=txGcDatabase.transaction('records','readwrite');
          txGcRefs.push(new WeakRef(tx));
          tx.objectStore('records').put('value-'+index,index);
          tx.oncomplete=()=>txGcEvents.push(index);
          tx.onabort=()=>txGcErrors.push(index);
        } 'queued'"#,
    )
    .unwrap();

    collect(&mut vm);
    assert_eq!(
        vm.eval("txGcRefs.length===4 && txGcRefs.every(ref=>ref.deref()!==undefined)")
            .unwrap(),
        "true"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(txGcEvents.length>0 || txGcErrors.length>0)",
        "true",
        "transaction GC first SQL commit",
    )
    .await;
    collect(&mut vm);
    assert_eq!(
        vm.eval("txGcErrors.length===0 && txGcRefs.every((ref,index)=>txGcEvents.includes(index) || ref.deref()!==undefined)")
            .unwrap(),
        "true"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(txGcEvents.length===4 || txGcErrors.length>0)",
        "true",
        "transaction GC all SQL commits",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(txGcErrors)").unwrap(), "[]");
    assert_eq!(vm.eval("JSON.stringify(txGcEvents)").unwrap(), "[0,1,2,3]");
    collect(&mut vm);
    assert_eq!(
        vm.eval("txGcRefs.every(ref=>ref.deref()===undefined)")
            .unwrap(),
        "true"
    );
    assert_eq!(native_entries(&mut vm), 0);

    vm.eval(
        r#"globalThis.txGcData=null;
        (()=>{
          const tx=txGcDatabase.transaction('records');
          const request=tx.objectStore('records').getAll();
          tx.oncomplete=()=>{txGcData=request.result;txGcDatabase.close();};
        })(); 'verifying'"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(txGcData!==null)",
        "true",
        "transaction GC committed data readback",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(txGcData)").unwrap(),
        r#"["value-0","value-1","value-2","value-3"]"#
    );
    collect(&mut vm);
    assert_eq!(native_entries(&mut vm), 0);
}
