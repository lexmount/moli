use super::*;

#[test]
fn indexed_db_transaction_durability_getters_preserve_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://indexeddb-durability-proxy.test/");
    vm.eval(
        r#"document.body.innerHTML='<iframe></iframe>';
      globalThis.childWindow=document.querySelector('iframe').contentWindow;
      const opening=indexedDB.open('durability-proxy',1);
      opening.onupgradeneeded=()=>opening.result.createObjectStore('s');
      opening.onsuccess=()=>globalThis.durabilityDatabase=opening.result;"#,
    )
    .unwrap();
    vm.eval_after_selected_page_tasks("'opened'").unwrap();
    vm.eval("globalThis.durabilityTransaction=durabilityDatabase.transaction('s','readonly',{durability:'strict'})").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "durabilityTransaction");
        let transaction =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, transaction, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::IDBTransaction::is_instance(
            scope,
            proxy.into()
        ));
        let key = crate::util::v8str(scope, "durabilityProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"globalThis.checkDurabilityProxy=()=>{
      for(const realm of [window,childWindow]) {
        const getter=Object.getOwnPropertyDescriptor(realm.IDBTransaction.prototype,'durability').get;
        if(getter.call(durabilityProxy)!=='strict') throw Error('native durability value');
        let traps=0;
        const author=new Proxy(durabilityProxy,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 43;}});
        const revoked=Proxy.revocable(durabilityProxy,{});revoked.revoke();
        for(const receiver of [author,revoked.proxy,Object.create(durabilityTransaction)]) {
          let error;try {getter.call(receiver);} catch(caught) {error=caught;}
          if(Object.getPrototypeOf(error)!==realm.TypeError.prototype || traps) throw Error('author durability receiver');
        }
      }
      return true;
    };checkDurabilityProxy()"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("checkDurabilityProxy()")
            .unwrap(),
        "true"
    );
    vm.eval("durabilityDatabase.close()").unwrap();
}

#[test]
fn indexed_db_durability_resolves_bucket_defaults_without_reading_author_shadows() {
    use moli_indexeddb::TransactionDurability::{Relaxed, Strict};
    let mut vm =
        new_storage_page_task_executor_test_vm("https://indexeddb-bucket-durability.test/");
    vm.eval(r#"
globalThis.durabilityBucketsReady = 'pending';
(async () => {
  const open = factory => new Promise((resolve, reject) => {
    const request = factory.open('records', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('s');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const namedDefault = await navigator.storageBuckets.open('default', {durability:'strict'});
  const strict = await navigator.storageBuckets.open('strict', {durability:'strict'});
  const relaxed = await navigator.storageBuckets.open('relaxed', {durability:'relaxed'});
  globalThis.durabilityBucketDatabases = await Promise.all([
    open(indexedDB), open(namedDefault.indexedDB), open(strict.indexedDB), open(relaxed.indexedDB)]);
})().then(() => durabilityBucketsReady = 'ready',
          error => durabilityBucketsReady = String(error));
"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("durabilityBucketsReady")
            .unwrap(),
        "ready"
    );
    vm.eval(
        r#"
globalThis.policyTransactions = [
  durabilityBucketDatabases[0].transaction('s', 'readwrite'),
  durabilityBucketDatabases[1].transaction('s', 'readwrite'),
  durabilityBucketDatabases[2].transaction('s', 'readwrite'),
  durabilityBucketDatabases[2].transaction('s', 'readwrite', {durability:'relaxed'}),
  durabilityBucketDatabases[3].transaction('s', 'readwrite'),
  durabilityBucketDatabases[3].transaction('s', 'readwrite', {durability:'strict'})];
for (const transaction of policyTransactions)
  Object.defineProperty(transaction, 'durability', {get() {throw Error('author shadow');}});
"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let transactions = global
            .get(
                scope,
                crate::util::v8str(scope, "policyTransactions").into(),
            )
            .unwrap();
        let transactions = v8::Local::<v8::Array>::try_from(transactions).unwrap();
        for (index, expected) in [Relaxed, Strict, Strict, Relaxed, Relaxed, Strict]
            .into_iter()
            .enumerate()
        {
            let transaction = transactions.get_index(scope, index as u32).unwrap();
            let transaction = v8::Local::<v8::Object>::try_from(transaction).unwrap();
            assert_eq!(
                crate::context_bootstrap::indexed_db_transaction_durability_for_test(
                    scope,
                    transaction
                )
                .unwrap(),
                expected,
                "transaction {index}"
            );
        }
        Ok(())
    })
    .unwrap();
    vm.eval_after_selected_page_tasks(
        "durabilityBucketDatabases.forEach(db => db.close()); 'done'",
    )
    .unwrap();
}
