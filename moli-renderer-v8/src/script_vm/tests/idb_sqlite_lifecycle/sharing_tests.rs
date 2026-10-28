use super::*;
use crate::context_bootstrap::IndexedDbMetadataSharingProbe;

fn probe(vm: &mut ScriptVm) -> IndexedDbMetadataSharingProbe {
    vm.with_default_context_scope(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let database = global
            .get(scope, crate::util::v8str(scope, "db").into())
            .unwrap();
        let database = v8::Local::<v8::Object>::try_from(database).unwrap();
        Ok(
            crate::context_bootstrap::indexed_db_metadata_sharing_probe_for_test(scope, database)
                .unwrap(),
        )
    })
    .unwrap()
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
    backend.drain(&mut vm, &loader).await;
    // The same schema must survive a normal commit: refreshing surfaces must
    // not replace all metadata with newly decoded native copies.
    assert_eq!(before.live_nodes(), [1, 1, 2]);
    assert_eq!(probe(&mut vm).live_nodes(), [1, 1, 2]);
    vm.eval("db.close(); delete globalThis.db; delete globalThis.upgrade")
        .unwrap();
    collect(&mut vm);
    assert_eq!(before.live_nodes(), [1, 1, 2]);
    vm.eval("delete globalThis.aliases; delete globalThis.indexAliases")
        .unwrap();
    collect(&mut vm);
    assert_eq!(before.live_handles(), [0, 0]);
    // The canonical database mirror belongs to the runtime table. Wrapper-owned
    // state is gone at GC; retiring the realm releases that final cache owner.
    drop(vm);
    assert_eq!(before.live_nodes(), [0, 0, 0]);
}

#[tokio::test]
async fn idb_sqlite_upgrade_metadata_uses_a_shared_snapshot_and_restores_aliases_on_abort() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"(() => {
        globalThis.events = [];
        globalThis.aliases = Array.from({length:32}, () => upgrade.objectStore('records'));
        globalThis.oldIndex = aliases[0].index('unique');
        globalThis.createdIndex = aliases[0].createIndex('later','group');
        upgrade.onabort = () => events.push('abort:' + upgrade.error);
        upgrade.abort();
        return 'queued';
    })()"#,
    )
    .unwrap();
    let during = probe(&mut vm);
    assert_eq!(during.live_nodes(), [2, 2, 2]);
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
