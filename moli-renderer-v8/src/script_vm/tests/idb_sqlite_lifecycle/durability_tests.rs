use super::*;
use moli_indexeddb::TransactionDurability;
use moli_storage_service::{IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME, StorageBucketDurability};

fn policy(vm: &mut ScriptVm, name: &'static str) -> TransactionDurability {
    let mut found = None;
    vm.with_default_context_scope(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let value = global
            .get(scope, crate::util::v8str(scope, name).into())
            .unwrap();
        let transaction = v8::Local::<v8::Object>::try_from(value).unwrap();
        found = crate::context_bootstrap::sql_transaction_durability_for_test(scope, transaction);
        Ok(())
    })
    .unwrap();
    found.expect("real transaction has a resolved native policy")
}

#[tokio::test]
async fn idb_sqlite_durability_snapshots_bucket_policy_before_admission_and_preserves_native_receivers()
 {
    let (mut vm, loader, backend) = fixture();
    let storage_key = String::from_utf8(backend.client.identity().owner_key.clone()).unwrap();
    backend
        .buckets
        .lock()
        .open_bucket_with_options(
            &storage_key,
            "default",
            None,
            Some(StorageBucketDurability::Strict),
            None,
            None,
        )
        .unwrap();
    vm.with_default_context_scope(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let function = v8::Function::new(scope, super::rename_tests::registered_proxy).unwrap();
        assert_eq!(
            global.set(
                scope,
                crate::util::v8str(scope, "nativeProxy").into(),
                function.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"
        globalThis.events=[];
        globalThis.first=db.transaction('records','readwrite');
        first.objectStore('records').put({group:'first'});
        first.oncomplete=()=>events.push('first:'+first.durability);
        globalThis.nativeFirst=nativeProxy(first);
        globalThis.nativeDb=nativeProxy(db);
        const getter=Object.getOwnPropertyDescriptor(IDBTransaction.prototype,'durability').get;
        globalThis.nativeHint=getter.call(nativeFirst);
        globalThis.rejected=false;
        try {getter.call(new Proxy(nativeFirst,{}));} catch(error) {rejected=error instanceof TypeError;}
        'queued'
    "#).unwrap();
    assert_eq!(policy(&mut vm, "first"), TransactionDurability::Relaxed);
    assert_eq!(
        vm.eval("[nativeHint,rejected].join(',')").unwrap(),
        "default,true"
    );
    // The first writer has not reached its SQL BEGIN yet. Updating the actual
    // implicit bucket now must only affect transactions created afterwards.
    backend
        .buckets
        .lock()
        .open_bucket_with_options(
            &storage_key,
            IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
            None,
            Some(StorageBucketDurability::Strict),
            None,
            None,
        )
        .unwrap();
    vm.eval(r#"
        globalThis.second=IDBDatabase.prototype.transaction.call(nativeDb,'records','readwrite');
        second.objectStore('records').put({group:'second'});
        second.oncomplete=()=>events.push('second:'+second.durability);
        Object.defineProperty(second,'durability',{configurable:true,get(){throw Error('author shadow');}});
        globalThis.relaxed=db.transaction('records','readwrite',{durability:'relaxed'});
        relaxed.objectStore('records').put({group:'relaxed'});
        relaxed.oncomplete=()=>events.push('relaxed:'+relaxed.durability);
        'queued'
    "#).unwrap();
    assert_eq!(policy(&mut vm, "first"), TransactionDurability::Relaxed);
    assert_eq!(policy(&mut vm, "second"), TransactionDurability::Strict);
    assert_eq!(policy(&mut vm, "relaxed"), TransactionDurability::Relaxed);
    backend
        .buckets
        .lock()
        .open_bucket_with_options(
            &storage_key,
            IMPLICIT_DEFAULT_BUCKET_INTERNAL_NAME,
            None,
            Some(StorageBucketDurability::Relaxed),
            None,
            None,
        )
        .unwrap();
    vm.eval(
        r#"
        delete second.durability;
        globalThis.strict=db.transaction('records','readwrite',{durability:'strict'});
        strict.objectStore('records').put({group:'strict'});
        strict.oncomplete=()=>events.push('strict:'+strict.durability);
        'queued'
    "#,
    )
    .unwrap();
    assert_eq!(policy(&mut vm, "second"), TransactionDurability::Strict);
    assert_eq!(policy(&mut vm, "strict"), TransactionDurability::Strict);
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["first:default","second:default","relaxed:relaxed","strict:strict"]"#
    );
    for (name, expected) in [
        ("first", TransactionDurability::Relaxed),
        ("second", TransactionDurability::Strict),
        ("relaxed", TransactionDurability::Relaxed),
        ("strict", TransactionDurability::Strict),
    ] {
        assert_eq!(policy(&mut vm, name), expected);
    }
    vm.eval("globalThis.count=-1;db.transaction('records').objectStore('records').count().onsuccess=e=>count=e.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(count)").unwrap(), "4");
}
