use super::*;

#[test]
fn indexed_db_sqlite_renames_preserve_utf16_names_records_and_generators_after_restart() {
    let root = indexed_db_test_root("schema-rename-profile-restart");
    for write in [true, false] {
        let manager = crate::new_indexed_db_manager(Some(root.clone())).unwrap();
        let mut vm =
            new_storage_page_task_executor_test_vm("https://indexeddb-schema-rename-profile.test/");
        vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));
        vm.set_storage_bucket_store(
            crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager),
        );
        vm.eval(&format!("globalThis.writeRename = {write};"))
            .unwrap();
        vm.eval(r#"
globalThis.renameProfile = 'pending';
(async () => {
  const S = '\uD800', I = '\uD801';
  const result = r => new Promise((resolve,reject) => {
    r.onsuccess=()=>resolve(r.result); r.onerror=()=>reject(r.error);
  });
  const complete = tx => new Promise((resolve,reject) => {
    tx.oncomplete=resolve; tx.onabort=()=>reject(tx.error);
  });
  let initialUpgrade, abortedUpgrade;
  const sameScope=tx=>JSON.stringify(Array.from(tx.objectStoreNames))===JSON.stringify([S]);
  const open = indexedDB.open('renames',1);
  open.onupgradeneeded=()=> {
    if(!writeRename) throw Error('persisted database disappeared');
    initialUpgrade=open.transaction;
    const store=open.result.createObjectStore('before',{autoIncrement:true});
    const index=store.createIndex('before-index','tag',{unique:true});
    store.add({tag:'one'});
    index.name=I;
    store.name=S;
    store.add({tag:'two'});
  };
  let db=await result(open);
  if(writeRename) {
    if(!sameScope(initialUpgrade)) throw Error('finished upgrade scope');
    db.close();
    const upgrade=indexedDB.open('renames',2);
    upgrade.onupgradeneeded=()=> {
      abortedUpgrade=upgrade.transaction;
      const store=upgrade.transaction.objectStore(S);
      store.index(I).name='aborted-index';store.name='aborted-store';
      upgrade.transaction.abort();
      if(store.name!==S || store.indexNames[0]!==I) throw Error('synchronous rename rollback');
    };
    await new Promise((resolve,reject)=> {
      upgrade.onerror=e=>{e.preventDefault();upgrade.error.name==='AbortError'?resolve():reject(upgrade.error);};
      upgrade.onsuccess=()=>reject(Error('aborted rename committed'));
    });
    if(!sameScope(abortedUpgrade)) throw Error('finished aborted upgrade scope');
    db=await result(indexedDB.open('renames',1));
  }
  const tx=db.transaction(S,'readwrite');const done=complete(tx);
  const store=tx.objectStore(S), index=store.index(I);
  const values=result(store.getAll()), keys=result(index.getAllKeys());
  const next=result(store.add({tag:writeRename?'three':'four'}));
  const actual=[db.version,store.name===S,index.name===I,index.objectStore===store,
    await values,await keys,await next];
  await done;if(!sameScope(tx)) throw Error('finished ordinary scope');db.close();return actual;
})().then(value=>renameProfile=JSON.stringify(value),error=>renameProfile=String(error));
"#).unwrap();
        let actual = vm.eval_after_selected_page_tasks("renameProfile").unwrap();
        let expected = if write {
            serde_json::json!([1,true,true,true,[{"tag":"one"},{"tag":"two"}],[1,2],3])
        } else {
            serde_json::json!([1,true,true,true,[{"tag":"one"},{"tag":"two"},{"tag":"three"}],[1,3,2],4])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            expected,
            "profile write: {write}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
