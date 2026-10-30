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

fn transaction_resources(vm: &mut ScriptVm) -> [usize; 3] {
    vm.with_default_context_scope(|scope, _| {
        Ok(crate::context_bootstrap::indexed_db_transaction_resources_for_test(scope))
    })
    .unwrap()
}

#[tokio::test]
async fn idb_sqlite_finished_transactions_release_clients_but_keep_retained_web_state() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(r#"(() => {
        globalThis.transactionHandles = [];
        globalThis.transactionEvents = [];
        globalThis.keepTransaction = (tx, outcome) => {
            const store = tx.objectStore('records');
            transactionHandles.push({tx, store, index:store.index('unique'), outcome, mode:tx.mode});
            tx.oncomplete = () => transactionEvents.push('complete');
            tx.onabort = () => transactionEvents.push('abort');
            return store;
        };
        keepTransaction(upgrade, 'complete');
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    for action in ["readonly", "commit", "abort", "running-abort", "failure"] {
        vm.eval(&format!("globalThis.transactionAction = {action:?}"))
            .unwrap();
        vm.eval(r#"(() => {
            const action = transactionAction;
            for (let i = 0; i < 8; i++) {
                const tx = db.transaction('records', action === 'readonly' ? 'readonly' : 'readwrite');
                const outcome = action === 'readonly' || action === 'commit' ? 'complete' : 'abort';
                const store = keepTransaction(tx, outcome);
                if (action === 'readonly') store.count();
                else if (action === 'abort') tx.abort();
                else if (action === 'running-abort') {
                    store.put({id:1000+i,group:'abort-'+i}).onsuccess = () => tx.abort();
                } else if (action === 'failure') store.add({id:i+1,group:'duplicate-'+i});
                else {
                    store.add({id:i+1,group:'committed-'+i});
                    tx.commit();
                }
            }
        return 'queued';
    })()"#).unwrap();
        backend.drain(&mut vm, &loader).await;
    }
    assert_eq!(vm.eval(r#"(() => {
        const throws = (callback, name) => {
            try { callback(); } catch (error) {
                if (error.name === name) return;
                throw error;
            }
            throw Error('finished transaction accepted an operation');
        };
        if (transactionEvents.length !== transactionHandles.length ||
            transactionEvents.filter(event => event === 'abort').length !== 24) throw Error('terminal events');
        for (const {tx,store,index,outcome,mode} of transactionHandles) {
            if (tx.db !== db || tx.mode !== mode || Array.from(tx.objectStoreNames).join(',') !== 'records' ||
                store.transaction !== tx || store.name !== 'records' || store.keyPath !== 'id' ||
                index.objectStore !== store || index.name !== 'unique' || index.keyPath !== 'group')
                throw Error('retained Web state');
            if (outcome === 'complete' && tx.error !== null) throw Error('commit error');
            throws(() => store.get(1), 'TransactionInactiveError');
            throws(() => index.get('x'), 'TransactionInactiveError');
            throws(() => tx.abort(), 'InvalidStateError');
            throws(() => tx.commit(), 'InvalidStateError');
        }
        return true;
    })()"#).unwrap(), "true");
    // The wrappers deliberately remain reachable. Native clients must be
    // released by the terminal path without waiting for GC or realm teardown.
    assert_eq!(transaction_resources(&mut vm), [41, 0, 0]);
}

#[tokio::test]
async fn idb_sqlite_finished_transactions_collect_and_release_old_metadata_in_a_live_realm() {
    let (mut vm, loader, backend) = fixture();
    let metadata = std::sync::Arc::downgrade(&backend.client.committed().unwrap());
    for _ in 0..4 {
        vm.eval(
            r#"(() => {
            for (let i = 0; i < 32; i++) db.transaction('records').objectStore('records').count();
            return 'queued';
        })()"#,
        )
        .unwrap();
        backend.drain(&mut vm, &loader).await;
        collect(&mut vm);
        assert_eq!(transaction_resources(&mut vm), [0, 0, 0]);
    }
    vm.eval("db.close(); delete globalThis.db").unwrap();
    backend.drain(&mut vm, &loader).await;
    collect(&mut vm);
    // Sweep the database's small registry entry after GC, then delete and
    // replace this SQL generation while keeping the realm alive.
    assert_eq!(transaction_resources(&mut vm), [0, 0, 0]);
    backend.execute(Command::DeleteDatabase);
    let (sender, receiver) = mpsc::channel();
    let replacement = backend
        ._service
        .resolve(
            backend.client.identity().owner_key.clone(),
            "sql-web-lifecycle".into(),
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .result
        .unwrap();
    assert_ne!(replacement.id(), backend.client.id());
    drop(replacement);
    drop(backend);
    assert!(
        metadata.upgrade().is_none(),
        "finished transactions pinned old SQL metadata"
    );
    assert_eq!(vm.eval("String(1 + 1)").unwrap(), "2");
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
            indexAliases[0].keyPath === indexAliases[1].keyPath,
            indexAliases[0].keyPath !== indexAliases[64].keyPath,
            aliases.slice(0,64).every(store => store === aliases[0]),
            aliases.slice(64).every(store => store === aliases[64]),
            aliases[0] !== aliases[64],
            indexAliases.slice(0,64).every(index => index === indexAliases[0]),
            indexAliases.slice(64).every(index => index === indexAliases[64])].join(',');
    })()"#).unwrap(), "true,true,true,true,true,true,true,true,true");
    let before = probe(&mut vm);
    assert_eq!([before.store_handles, before.index_handles], [2, 2]);
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
    // The database mirror is also wrapper-owned: no realm teardown or another
    // runtime-table lookup is required to release the final shared schema.
    assert_eq!(before.live_nodes(), [0, 0, 0]);
}

#[tokio::test]
async fn idb_sqlite_upgrade_metadata_uses_a_shared_snapshot_and_restores_aliases_on_abort() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    let during = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            // Inspect shared definitions within one body, before the checkpoint can
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
async fn idb_sqlite_deleted_index_definition_is_released_at_last_wrapper_gc() {
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
    let definitions = probe(&mut vm);
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
        backend.client.committed().unwrap().stores[&moli_indexeddb::IndexedDbName::from("records")]
            .indexes
            .len(),
        1
    );
    assert_eq!(definitions.live_nodes()[2], 2);
    vm.eval("delete globalThis.deletedIndex").unwrap();
    collect(&mut vm);
    assert_eq!(
        definitions.live_nodes()[2],
        1,
        "a deleted index owns its definition until its final wrapper is collected"
    );
}
