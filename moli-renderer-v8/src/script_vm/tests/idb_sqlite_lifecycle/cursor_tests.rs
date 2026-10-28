use super::*;

async fn seed() -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    crate::network::ResourceRequestClientOwner,
    SqlBackend,
) {
    let (mut vm, loader, backend) = fixture();
    vm.eval(
        r#"(() => {
        const store = db.transaction('records','readwrite').objectStore('records');
        for (const [id, tag] of [[1,'a'],[2,'b'],[3,'a'],[4,'c'],[5,'b'],[6,'c']]) {
            store.put({id,group:'g'+id,tag});
        }
        return 'queued';
    })()"#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    (vm, loader, backend)
}

async fn index(
    backend: &SqlBackend,
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) {
    backend.upgrade(vm);
    vm.eval("upgrade.objectStore('records').createIndex('tag','tag'); 'queued'")
        .unwrap();
    backend.drain(vm, loader).await;
}

#[tokio::test]
async fn idb_sqlite_cursor_iterates_one_row_and_preserves_pending_and_identity() {
    let (mut vm, loader, backend) = seed().await;
    vm.eval(r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        const request = store.openCursor(IDBKeyRange.bound(2,4));
        let first;
        request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor) { events.push('end'); return; }
            if (!first) first = cursor;
            events.push([cursor.key,cursor.primaryKey,cursor.value.id,cursor===first,cursor.source===store,cursor.request===request].join(':'));
            const value = cursor.value;
            cursor.continue();
            events.push(cursor.value===value && cursor.key===value.id);
            const errors = [];
            for (const run of [()=>cursor.continue(),()=>cursor.advance(1),()=>cursor.update(value),()=>cursor.delete()]) {
                try { run(); errors.push('accepted'); } catch(error) { errors.push(error.name); }
            }
            events.push(errors.join(','));
        };
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('abort:' + tx.error?.name);
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["2:2:2:true:true:true",true,"InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError","3:3:3:true:true:true",true,"InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError","4:4:4:true:true:true",true,"InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError","end","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_cursor_uses_tuple_seek_and_unique_group_order_in_all_directions() {
    let (mut vm, loader, backend) = seed().await;
    index(&backend, &mut vm, &loader).await;
    vm.eval(
        r#"
        globalThis.found = {};
        const tx = db.transaction('records');
        const source = tx.objectStore('records').index('tag');
        for (const direction of ['next','prev','nextunique','prevunique']) {
            const rows = found[direction] = [];
            const request = source.openKeyCursor(null,direction);
            request.onsuccess = () => {
                const cursor = request.result;
                if (!cursor) return;
                rows.push(cursor.key + ':' + cursor.primaryKey);
                cursor.continue();
            };
        }
        tx.oncomplete = () => found.complete = true;
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(found)").unwrap(),
        r#"{"next":["a:1","a:3","b:2","b:5","c:4","c:6"],"prev":["c:6","c:4","b:5","b:2","a:3","a:1"],"nextunique":["a:1","b:2","c:4"],"prevunique":["c:4","b:2","a:1"],"complete":true}"#
    );
}

#[tokio::test]
async fn idb_sqlite_cursor_seek_observes_mutations_and_advance_skips_without_a_snapshot() {
    let (mut vm, loader, backend) = seed().await;
    vm.eval(r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        const request = store.openCursor();
        request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor) { events.push('end'); return; }
            events.push(cursor.key + ':' + cursor.value.group);
            if (cursor.key===1) {
                store.delete(2);
                store.put({id:2.5,group:'inserted'});
                store.put({id:3,group:'changed'});
                cursor.continue();
            } else if (cursor.key===2.5) {
                cursor.advance(2);
            } else if (cursor.key===4) {
                cursor.update({id:4,group:'cursor-updated'}).onsuccess = event => events.push('updated:' + event.target.result);
                cursor.delete().onsuccess = () => events.push('deleted');
                store.put({id:7,group:'appended'});
                cursor.continue(6);
            } else cursor.continue();
        };
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('abort:' + tx.error?.name);
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["1:g1","2.5:inserted","4:g4","updated:4","deleted","6:g6","7:appended","end","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_cursor_continue_primary_key_validates_and_seeks_in_both_directions() {
    let (mut vm, loader, backend) = seed().await;
    index(&backend, &mut vm, &loader).await;
    vm.eval(r#"
        globalThis.found = {};
        const tx = db.transaction('records');
        const source = tx.objectStore('records').index('tag');
        for (const direction of ['next','prev']) {
            const rows = found[direction] = [];
            const request = source.openCursor(null,direction);
            request.onsuccess = () => {
                const cursor = request.result;
                if (!cursor) return;
                rows.push(cursor.key+':'+cursor.primaryKey);
                try { cursor.continuePrimaryKey(cursor.key,cursor.primaryKey); } catch(error) { rows.push(error.name); }
                if (direction==='next' && cursor.key==='a') cursor.continuePrimaryKey('b',4);
                else if (direction==='prev' && cursor.key==='c') cursor.continuePrimaryKey('b',4);
                else cursor.continue();
            };
        }
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(found)").unwrap(),
        r#"{"next":["a:1","DataError","b:5","DataError","c:4","DataError","c:6","DataError"],"prev":["c:6","DataError","b:2","DataError","a:3","DataError","a:1","DataError"]}"#
    );
}

#[tokio::test]
async fn idb_sqlite_cursor_keeps_the_first_value_getter_realm_and_cached_identity() {
    let (mut vm, loader, backend) = seed().await;
    vm.eval(r#"
        document.body.innerHTML = '<iframe id=child></iframe>';
        globalThis.events = [];
        const realm = document.getElementById('child').contentWindow;
        const getter = Object.getOwnPropertyDescriptor(realm.IDBCursorWithValue.prototype,'value').get;
        const tx = db.transaction('records');
        const request = tx.objectStore('records').openCursor(IDBKeyRange.only(1));
        request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor) { events.push('end'); return; }
            const value = getter.call(cursor);
            events.push(Object.getPrototypeOf(value)===realm.Object.prototype, cursor.value===value);
            cursor.continue();
            events.push(getter.call(cursor)===value);
        };
        tx.oncomplete = () => events.push('complete');
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[true,true,true,"end","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_cursor_iterates_more_data_than_the_queue_budget_without_snapshotting() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-cursor-budget.test/",
        &loader,
    );
    let mut config = ServiceConfig::default();
    config.executor.data.bytes = 64 * 1024;
    let backend = SqlBackend::with_config(config);
    backend.bind(&mut vm);
    for id in 1..=16 {
        vm.eval(&format!("db.transaction('records','readwrite').objectStore('records').put({{id:{id},group:'g{id}',payload:'x'.repeat(8192)}})")).unwrap();
        backend.drain(&mut vm, &loader).await;
    }
    vm.eval(
        r#"(() => {
        globalThis.rows = [];
        const request = db.transaction('records').objectStore('records').openCursor();
        request.onsuccess = () => {
            const cursor = request.result;
            if (cursor) { rows.push(cursor.key); cursor.continue(); }
        };
        return 'queued';
    })()"#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("rows.join(',')").unwrap(),
        "1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16"
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
}

#[tokio::test]
async fn idb_sqlite_cursor_lazy_row_reservation_survives_retention_and_releases_on_gc() {
    let (mut vm, loader, backend) = fixture();
    vm.eval("db.transaction('records','readwrite').objectStore('records').put({id:1,group:'g',payload:'x'.repeat(16384)})").unwrap();
    backend.drain(&mut vm, &loader).await;
    vm.eval(
        r#"(() => {
        const request = db.transaction('records').objectStore('records').openCursor();
        request.onsuccess = () => globalThis.heldCursor = request.result;
        return 'queued';
    })()"#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert!(backend._service.queue_usage().data_bytes > 16384);
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
        vm.eval("String(heldCursor.value.payload.length)").unwrap(),
        "16384"
    );
    assert!(backend._service.queue_usage().data_bytes > 16384);
    vm.eval("delete globalThis.heldCursor").unwrap();
    collect(&mut vm);
    backend.barrier();
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
}

#[tokio::test]
async fn idb_sqlite_key_cursor_never_decodes_wire_and_value_cursor_propagates_decode_failure() {
    let (mut vm, loader, backend) = fixture();
    let handle = backend.client.allocate_transaction_handle().unwrap();
    let admission = backend
        .client
        .queue_transaction_start(
            DatabaseHandle::from_raw((1u64 << 53) + 1),
            &["records".into()],
            TransactionMode::ReadWrite,
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
    backend.execute(Command::Begin {
        transaction: handle,
        mode: TransactionMode::ReadWrite,
        stores: BTreeSet::from(["records".into()]),
        version: None,
        admission: Some(admission.handle().clone()),
    });
    backend.execute(Command::Write {
        transaction: handle,
        store: "records".into(),
        key: Some(1.into()),
        value: vec![0xff].into(),
        indexes: vec![moli_indexeddb::sqlite::IndexKeys {
            name: "unique".into(),
            keys: vec!["g".into()],
        }],
        add_only: false,
        quota: None,
    });
    backend.execute(Command::Commit {
        transaction: handle,
    });
    vm.eval(r#"(() => {
        globalThis.found = {keys:[],values:[]};
        const store = db.transaction('records').objectStore('records');
        const keys = store.openKeyCursor();
        keys.onsuccess = () => {
            const cursor = keys.result;
            if (!cursor) { found.keys.push('end'); return; }
            found.keys.push(cursor.key, 'value' in cursor);
            cursor.continue();
        };
        const values = store.openCursor();
        values.onsuccess = () => {
            const cursor = values.result;
            if (!cursor) { found.values.push('end'); return; }
            found.values.push(cursor.key);
            try { void cursor.value; found.values.push('unexpected success'); }
            catch(error) { found.values.push(error instanceof Error || error instanceof DOMException); }
            cursor.continue();
        };
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(found)").unwrap(),
        r#"{"keys":[1,false,"end"],"values":[1,true,"end"]}"#
    );
}

fn terminate_listener(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.terminate_execution();
}

#[tokio::test]
async fn idb_sqlite_cursor_listener_termination_rolls_back_only_automatic_commit() {
    for explicit in [false, true] {
        let (mut vm, loader, backend) = seed().await;
        vm.with_default_context_scope(|scope, _| {
            let callback = v8::Function::new(scope, terminate_listener).unwrap();
            let global = scope.get_current_context().global(scope);
            global
                .set(
                    scope,
                    crate::util::v8str(scope, "terminateListener").into(),
                    callback.into(),
                )
                .unwrap();
            Ok(())
        })
        .unwrap();
        vm.eval(&format!(
            r#"(() => {{
            globalThis.events = [];
            const tx = db.transaction('records','readwrite');
            const store = tx.objectStore('records');
            store.put({{id:9,group:'pending'}});
            const request = store.openCursor();
            request.onsuccess = () => {{
                events.push('cursor');
                terminateListener();
                while (true) {{}}
            }};
            tx.oncomplete = () => events.push('complete');
            if ({explicit}) tx.commit();
            return 'queued';
        }})()"#
        ))
        .unwrap();
        let admission = backend
            .client
            .queue_transaction_start(
                DatabaseHandle::from_raw((1u64 << 53) + 1),
                &["records".into()],
                TransactionMode::ReadOnly,
                std::sync::Arc::new(|| {}),
            )
            .unwrap();
        for _ in 0..30 {
            backend.barrier();
            if admission.handle().is_ready() {
                break;
            }
            match vm
                .run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
            {
                Ok(true) => {}
                Ok(false) => panic!("terminated transaction stranded native admission"),
                Err(error) => {
                    let message = format!("{error:#}").to_lowercase();
                    assert!(
                        message.contains("terminat") || message.contains("interrupted"),
                        "{message}"
                    );
                }
            }
        }
        backend.barrier();
        assert!(
            admission.handle().is_ready(),
            "native finish must release admission after termination"
        );
        let handle = backend.client.allocate_transaction_handle().unwrap();
        backend.execute(Command::Begin {
            transaction: handle,
            mode: TransactionMode::ReadOnly,
            stores: BTreeSet::from(["records".into()]),
            version: None,
            admission: Some(admission.handle().clone()),
        });
        let CommandResult::Count(count) = backend.execute(Command::Count {
            transaction: handle,
            store: "records".into(),
            source: Default::default(),
            range: Default::default(),
        }) else {
            panic!("native count after termination");
        };
        assert_eq!(count, if explicit { 7 } else { 6 });
        backend.execute(Command::Commit {
            transaction: handle,
        });
        backend.drain(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            if explicit {
                r#"["cursor","complete"]"#
            } else {
                r#"["cursor"]"#
            }
        );
    }
}
