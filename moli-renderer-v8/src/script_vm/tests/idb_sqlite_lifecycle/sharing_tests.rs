use super::*;
use crate::context_bootstrap::IndexedDbMetadataSharingProbe;

fn probe(vm: &mut ScriptVm) -> IndexedDbMetadataSharingProbe {
    vm.with_default_context_scope(|scope, _| Ok(probe_in_scope(scope)))
        .unwrap()
}

fn probe_in_scope(scope: &mut v8::PinScope<'_, '_>) -> IndexedDbMetadataSharingProbe {
    let global = scope.get_current_context().global(scope);
    let database = global
        .get(scope, crate::util::v8str(scope, "db").into())
        .unwrap();
    let database = v8::Local::<v8::Object>::try_from(database).unwrap();
    crate::context_bootstrap::indexed_db_metadata_sharing_probe_for_test(scope, database).unwrap()
}

fn collect(vm: &mut crate::runtime::PageVmTaskExecutorTestHarness) {
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn idb_sqlite_metadata_is_shared_across_aliases_realms_and_readonly_commit() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval("upgrade.objectStore('records').createIndex('compound',['group','id'])")
        .unwrap();
    backend.drain(&mut vm, &loader).await;
    collect(&mut vm);
    let definition_bytes = backend.client.metadata_bytes();
    assert_eq!(vm.eval(r#"(() => {
        document.body.innerHTML = '<iframe id=child></iframe>';
        const realm = document.getElementById('child').contentWindow;
        const tx = db.transaction('records');
        globalThis.aliases = Array.from({length:64}, () => tx.objectStore('records'))
            .concat(Array.from({length:64}, () => realm.IDBTransaction.prototype.objectStore.call(tx,'records')));
        globalThis.indexAliases = aliases.map(store => store.index('compound'));
        return [Object.getPrototypeOf(indexAliases[0].keyPath) === Array.prototype,
            Object.getPrototypeOf(indexAliases[64].keyPath) === realm.Array.prototype,
            indexAliases[0].keyPath !== indexAliases[1].keyPath].join(',');
    })()"#).unwrap(), "true,true,true");
    let before = probe(&mut vm);
    assert_eq!([before.store_handles, before.index_handles], [128, 128]);
    assert_eq!(before.live_nodes(), [1, 1, 2]);
    assert_eq!(backend.client.metadata_bytes(), definition_bytes);
    backend.drain(&mut vm, &loader).await;
    // The same schema must survive a normal commit: refreshing surfaces must
    // not replace all metadata with newly decoded native copies.
    assert_eq!(before.live_nodes(), [1, 1, 2]);
    assert_eq!(probe(&mut vm).live_nodes(), [1, 1, 2]);
    assert_eq!(backend.client.metadata_bytes(), definition_bytes);
    vm.eval("db.close(); delete globalThis.db; delete globalThis.upgrade")
        .unwrap();
    collect(&mut vm);
    assert_eq!(before.live_nodes(), [1, 1, 2]);
    vm.eval("delete globalThis.aliases; delete globalThis.indexAliases")
        .unwrap();
    collect(&mut vm);
    assert_eq!(before.live_handles(), [0, 0]);
    // The database mirror is also wrapper-owned: no realm teardown or another
    // runtime-table lookup is required to release the final schema leases.
    assert_eq!(before.live_nodes(), [0, 0, 0]);
}

#[tokio::test]
async fn idb_sqlite_upgrade_metadata_uses_a_shared_snapshot_and_restores_aliases_on_abort() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    let during = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            // Inspect the lease counts within one body, before the checkpoint can
            // auto-commit the upgrade between the metadata probe and abort().
            let source = v8::String::new(
                scope,
                r#"(() => {
        globalThis.events = [];
        globalThis.aliases = Array.from({length:32}, () => upgrade.objectStore('records'));
        globalThis.oldIndex = aliases[0].index('unique');
        globalThis.createdIndex = aliases[0].createIndex('later','group');
        upgrade.onabort = () => events.push('abort:' + upgrade.error);
        return 'queued';
    })()"#,
            )
            .unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            crate::script_execution::execute_compiled_script(scope, script).unwrap();
            let during = probe_in_scope(scope);
            assert_eq!(during.live_nodes(), [2, 2, 2]);
            let source = v8::String::new(scope, "upgrade.abort()").unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            crate::script_execution::execute_compiled_script(scope, script).unwrap();
            Ok(during)
        })
        .unwrap();
    // abort() restores every alias immediately, before SQL rollback finishes
    // and the abort event can be delivered. The extra index wrapper stays live.
    assert_eq!(during.live_nodes(), [1, 1, 2]);
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
    assert_eq!(
        vm.eval(
            "String(aliases.every(store => Array.from(store.indexNames).join(',') === 'unique'))"
        )
        .unwrap(),
        "true"
    );
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["abort:null"]"#
    );
    assert_eq!(
        vm.eval(
            "String(aliases.every(store => Array.from(store.indexNames).join(',') === 'unique'))"
        )
        .unwrap(),
        "true"
    );
    assert_eq!(vm.eval("String(oldIndex.name)").unwrap(), "unique");
    assert_eq!(
        vm.eval("(() => { try { createdIndex.get('x'); } catch(error) { return error.name; } })()")
            .unwrap(),
        "InvalidStateError"
    );
    assert_eq!(during.live_nodes(), [1, 1, 2]);
    vm.eval("delete globalThis.createdIndex").unwrap();
    collect(&mut vm);
    assert_eq!(during.live_nodes(), [1, 1, 1]);
    assert_eq!(backend.client.committed().unwrap().version, 1);
}

#[tokio::test]
async fn idb_sqlite_deleted_index_definition_lease_is_refunded_at_last_wrapper_gc() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"(() => {
        const store = upgrade.objectStore('records');
        globalThis.deletedIndex = store.createIndex('x'.repeat(32 * 1024), 'group');
        return 'queued';
    })()"#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    collect(&mut vm);
    backend.upgrade(&mut vm);
    vm.eval("upgrade.objectStore('records').deleteIndex('x'.repeat(32 * 1024))")
        .unwrap();
    backend.drain(&mut vm, &loader).await;
    collect(&mut vm);
    assert_eq!(
        vm.eval("String(deletedIndex.name.length)").unwrap(),
        "32768"
    );
    assert_eq!(
        vm.eval("(() => { try { deletedIndex.get(1); } catch(error) { return error.name; } })()")
            .unwrap(),
        "InvalidStateError"
    );
    assert_eq!(
        backend.client.committed().unwrap().stores["records"]
            .indexes
            .len(),
        1
    );
    let held_bytes = backend.client.metadata_bytes();
    vm.eval("delete globalThis.deletedIndex").unwrap();
    collect(&mut vm);
    assert!(
        held_bytes - backend.client.metadata_bytes() >= 32 * 1024,
        "a deleted index owns its definition until its final wrapper is collected"
    );
}
