use super::*;
use moli_indexeddb::sqlite::IndexKeys;
use moli_indexeddb::{IndexedDbError, IndexedDbExternalObject, IndexedDbValue, ReadBudget};
use v8::ValueSerializerHelper;

fn value(vm: &mut ScriptVm, code: &str) -> IndexedDbValue {
    vm.with_default_context_scope(|scope, _| {
        let source = v8::String::new(scope, code).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        let value = crate::script_execution::execute_compiled_script(scope, script).unwrap();
        Ok(crate::context_bootstrap::serialize_indexed_db_value_for_test(scope, value).unwrap())
    })
    .unwrap()
}

#[derive(Default)]
struct DecodeLimit {
    limit: usize,
    used: usize,
    charges: Vec<usize>,
}
impl ReadBudget for DecodeLimit {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        let bytes = usize::try_from(bytes).unwrap();
        self.charges.push(bytes);
        if bytes > self.limit - self.used {
            return Err(IndexedDbError::Resource(
                "test decode capacity exhausted".into(),
            ));
        }
        self.used += bytes;
        Ok(())
    }
    fn refund(&mut self, bytes: u64) {
        self.used -= bytes as usize;
    }
}

#[test]
fn idb_sqlite_blob_decode_charges_only_actual_host_occurrences_and_preserves_identity() {
    let (mut vm, _loader, _backend) = fixture();
    let saved = value(
        &mut vm,
        "(() => { const blob = new Blob(['body'],{type:'text/plain'}); return [blob,blob,new File(['file'],'saved.txt',{type:'text/plain',lastModified:123})]; })()",
    );
    assert_eq!(saved.external_objects().len(), 2);
    let mut budget = DecodeLimit {
        limit: 4096,
        ..Default::default()
    };
    vm.with_default_context_scope(|scope, _| {
        let decoded = crate::context_bootstrap::deserialize_indexed_db_value_for_test(
            scope,
            &saved,
            &mut budget,
        )
        .unwrap()
        .unwrap();
        let array = v8::Local::<v8::Array>::try_from(decoded).unwrap();
        assert_eq!(
            array.get_index(scope, 0).unwrap(),
            array.get_index(scope, 1).unwrap()
        );
        let global = scope.get_current_context().global(scope);
        global
            .set(scope, crate::util::v8str(scope, "decoded").into(), decoded)
            .unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        budget.charges,
        vec![
            std::mem::size_of::<moli_file_api::BlobId>(),
            4 + 10 + 2 * std::mem::size_of::<usize>(),
            std::mem::size_of::<moli_file_api::BlobId>(),
            4 + 10 + 9 + 2 * std::mem::size_of::<usize>()
        ]
    );
    assert_eq!(
        vm.eval(
            "[decoded[0].size,decoded[0].type,decoded[2].name,decoded[2].lastModified].join('|')"
        )
        .unwrap(),
        "4|text/plain|saved.txt|123"
    );
    // The extra unreferenced slot is deliberately never materialized or charged.
    let mut objects = saved.external_objects().to_vec();
    objects.push(IndexedDbExternalObject::Blob {
        bytes: vec![0; 1024 * 1024],
        mime_type: String::new(),
    });
    let saved = IndexedDbValue::new(saved.wire_bytes().to_vec(), objects);
    let mut second = DecodeLimit {
        limit: 4096,
        ..Default::default()
    };
    vm.with_default_context_scope(|scope, _| {
        assert!(
            crate::context_bootstrap::deserialize_indexed_db_value_for_test(
                scope,
                &saved,
                &mut second
            )
            .unwrap()
            .is_some()
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(budget.charges, second.charges);
}

struct RepeatedSlot;
impl v8::ValueSerializerImpl for RepeatedSlot {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
    }
    fn has_custom_host_object(&self, _isolate: &v8::Isolate) -> bool {
        true
    }
    fn is_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<bool> {
        Some(crate::web_api_interfaces::Blob::is_instance(scope, object))
    }
    fn write_host_object<'s>(
        &self,
        _scope: &mut v8::PinScope<'s, '_>,
        _object: v8::Local<'s, v8::Object>,
        serializer: &dyn v8::ValueSerializerHelper,
    ) -> Option<bool> {
        serializer.write_uint32(crate::structured_clone::HOST_OBJECT_TAG_BLOB);
        serializer.write_uint32(0);
        Some(true)
    }
}

#[test]
fn idb_sqlite_blob_decode_repeated_external_slot_cannot_bypass_capacity() {
    let (mut vm, _loader, _backend) = fixture();
    vm.with_default_context_scope(|scope, _| {
        let script = v8::Script::compile(
            scope,
            crate::util::v8str(scope, "[new Blob(['a']),new Blob(['b'])]"),
            None,
        )
        .unwrap();
        let input = crate::script_execution::execute_compiled_script(scope, script).unwrap();
        let wire = {
            let serializer = v8::ValueSerializer::new(scope, Box::new(RepeatedSlot));
            serializer.write_header();
            assert_eq!(
                serializer.write_value(scope.get_current_context(), input),
                Some(true)
            );
            serializer.release()
        };
        let saved = IndexedDbValue::new(
            wire,
            vec![IndexedDbExternalObject::Blob {
                bytes: vec![7; 1024],
                mime_type: String::new(),
            }],
        );
        let mut budget = DecodeLimit {
            limit: 1100,
            ..Default::default()
        };
        v8::tc_scope!(let scope, scope);
        let live = crate::blob::live_blob_count_for_test();
        let error = crate::context_bootstrap::deserialize_indexed_db_value_for_test(
            scope,
            &saved,
            &mut budget,
        )
        .unwrap_err();
        assert_eq!(
            crate::blob::live_blob_count_for_test(),
            live,
            "failed decode releases its partial Blob before GC"
        );
        assert!(matches!(error, IndexedDbError::Resource(_)));
        let copy = 1024 + 2 * std::mem::size_of::<usize>();
        assert_eq!(
            budget.charges,
            vec![
                std::mem::size_of::<moli_file_api::BlobId>(),
                copy,
                std::mem::size_of::<moli_file_api::BlobId>(),
                copy
            ]
        );
        assert_eq!(budget.used, 0);
        assert!(budget.used <= budget.limit);
        Ok(())
    })
    .unwrap();
}

fn bounded_fixture() -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    crate::network::ResourceRequestClientOwner,
    SqlBackend,
) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-blob-budget.test/",
        &loader,
    );
    let mut config = ServiceConfig::default();
    config.executor.data.bytes = 48 * 1024;
    let backend = SqlBackend::with_config(config);
    backend.bind(&mut vm);
    (vm, loader, backend)
}

fn seed_large_blob(vm: &mut ScriptVm, backend: &SqlBackend) {
    seed_blobs(vm, backend, 36 * 1024, 1);
}

fn seed_blobs(vm: &mut ScriptVm, backend: &SqlBackend, bytes: usize, count: usize) {
    let transaction = backend.client.allocate_transaction_handle().unwrap();
    let admission = backend
        .client
        .queue_transaction_start(
            DatabaseHandle::from_raw(7),
            &["records".into()],
            TransactionMode::ReadWrite,
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
    backend.execute(Command::Begin {
        transaction,
        mode: TransactionMode::ReadWrite,
        stores: BTreeSet::from(["records".into()]),
        version: None,
        admission: Some(admission.handle().clone()),
    });
    for id in 1..=count {
        let stored = value(
            vm,
            &format!(
                "(() => {{ const blob = new Blob([new Uint8Array({bytes})]); (globalThis.seeds ??= []).push(blob); return {{id:{id},group:'g{id}',blob}}; }})()"
            ),
        );
        backend.execute(Command::Write {
            transaction,
            store: "records".into(),
            key: Some(moli_indexeddb::Key::number(id as f64).unwrap()),
            value: stored,
            indexes: vec![IndexKeys {
                name: "unique".into(),
                keys: vec![moli_indexeddb::Key::from(format!("g{id}"))],
            }],
            add_only: false,
            quota: None,
        });
    }
    backend.execute(Command::Commit { transaction });
}

#[tokio::test]
async fn idb_sqlite_blob_read_capacity_failure_is_a_preventable_request_error() {
    for expression in [
        "store.get(1)",
        "store.getAll()",
        "store.getAllRecords()",
        "store.index('unique').get('g1')",
        "store.index('unique').getAllRecords()",
    ] {
        let (mut vm, loader, backend) = bounded_fixture();
        seed_large_blob(&mut vm, &backend);
        vm.eval(&format!(
            r#"(() => {{
            globalThis.events = [];
            const tx = db.transaction('records');
            const store = tx.objectStore('records');
            const request = {expression};
            request.onsuccess = () => events.push('bad-success');
            request.onerror = event => {{
                event.preventDefault();
                events.push('error:' + request.error.name);
                store.getKey(1).onsuccess = event => events.push('key:' + event.target.result);
            }};
            tx.oncomplete = () => events.push('complete');
            tx.onabort = () => events.push('bad-abort');
            return 'queued';
        }})()"#
        ))
        .unwrap();
        backend.drain(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["error:UnknownError","key:1","complete"]"#,
            "{expression}"
        );
        assert_eq!(backend._service.queue_usage().data_bytes, 0);
        assert_eq!(backend._service.queue_usage().control_tasks, 0);
    }
}

#[tokio::test]
async fn idb_sqlite_blob_cursor_lazy_decode_failure_keeps_position_and_releases_page_on_gc() {
    let (mut vm, loader, backend) = bounded_fixture();
    seed_large_blob(&mut vm, &backend);
    vm.eval(r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records');
        tx.objectStore('records').openCursor().onsuccess = event => {
            globalThis.cursor = event.target.result;
            for(let attempt=0;attempt<2;attempt++) {
                try { cursor.value; events.push('bad-value'); } catch(error) { events.push('error:' + error.name); }
            }
            events.push('key:' + cursor.key);
        };
        tx.oncomplete = () => events.push('complete');
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["error:UnknownError","error:UnknownError","key:1","complete"]"#
    );
    assert!(backend._service.queue_usage().data_bytes >= 36 * 1024);
    vm.eval("delete globalThis.cursor").unwrap();
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}

#[tokio::test]
async fn idb_sqlite_blob_inline_decode_capacity_failure_rolls_back_generated_key() {
    let (mut vm, loader, backend) = bounded_fixture();
    vm.eval(r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'large',blob:new Blob([new Uint8Array(36*1024)])}).onerror = event => {
            event.preventDefault();
            events.push('error:' + event.target.error.name);
            store.put({group:'small',blob:new Blob(['small'])}).onsuccess = event => events.push('key:' + event.target.result);
        };
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('bad-abort');
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["error:UnknownError","key:1","complete"]"#
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}

#[tokio::test]
async fn idb_sqlite_blob_backfill_decode_capacity_failure_restores_schema_and_record() {
    let (mut vm, loader, backend) = bounded_fixture();
    seed_large_blob(&mut vm, &backend);
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.events = [];
        upgrade.objectStore('records').createIndex('blob-size','blob.size');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        upgrade.oncomplete = () => events.push('bad-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["abort:UnknownError"]"#
    );
    let metadata = backend.client.committed().unwrap();
    assert_eq!(metadata.version, 1);
    assert_eq!(metadata.stores["records"].indexes.len(), 1);
    vm.eval("db.transaction('records').objectStore('records').count().onsuccess = event => events.push('count:' + event.target.result); 'queued'").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["abort:UnknownError","count:1"]"#
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
}

#[tokio::test]
async fn idb_sqlite_delivered_blob_and_file_outlive_record_clear_and_database_close() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(r#"(() => {
        const tx = db.transaction('records','readwrite');
        tx.objectStore('records').put({id:1,group:'g1',blob:new Blob(['blob-data'],{type:'text/plain'}),file:new File(['file-data'],'saved.txt',{type:'text/plain',lastModified:123})});
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    vm.eval(r#"(() => {
        const tx = db.transaction('records');
        tx.objectStore('records').getAllRecords().onsuccess = event => globalThis.kept = event.target.result[0].value;
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("[kept.blob.size,kept.blob.type,kept.file.name,kept.file.lastModified].join('|')")
            .unwrap(),
        "9|text/plain|saved.txt|123"
    );
    vm.eval("db.transaction('records','readwrite').objectStore('records').clear(); 'queued'")
        .unwrap();
    backend.drain(&mut vm, &loader).await;
    backend.execute(Command::Close);
    vm.eval("globalThis.contents=[]; kept.blob.text().then(text=>contents.push(text)); kept.file.text().then(text=>contents.push(text)); 'queued'").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(contents)").unwrap(),
        r#"["blob-data","file-data"]"#
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
}

#[tokio::test]
async fn idb_sqlite_internal_blob_decodes_release_backings_without_waiting_for_gc() {
    let (mut vm, loader, backend) = fixture();
    vm.eval("globalThis.inputBlob = new Blob([new Uint8Array(4096)]); 'ready'")
        .unwrap();
    let before = crate::blob::live_blob_count_for_test();
    assert_eq!(before, 1);
    vm.eval(r#"(() => {
        globalThis.written = 0;
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        for(let id=0;id<4;id++) store.put({group:'g'+id,blob:inputBlob}).onsuccess = () => written++;
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(written)").unwrap(), "4");
    assert_eq!(
        crate::blob::live_blob_count_for_test(),
        before,
        "synchronous key validation and asynchronous inline/index extraction release their temporary Blob backings"
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
}

#[tokio::test]
async fn idb_sqlite_partial_blob_collection_failure_releases_earlier_rows_without_gc() {
    let (mut vm, loader, backend) = bounded_fixture();
    seed_blobs(&mut vm, &backend, 14 * 1024, 2);
    let before = crate::blob::live_blob_count_for_test();
    assert_eq!(before, 2);
    vm.eval(r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records');
        const request = tx.objectStore('records').getAll();
        request.onsuccess = () => events.push('bad-success');
        request.onerror = event => { event.preventDefault(); events.push('error:' + request.error.name); };
        tx.oncomplete = () => events.push('complete');
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["error:UnknownError","complete"]"#
    );
    assert_eq!(
        crate::blob::live_blob_count_for_test(),
        before,
        "the successfully decoded first row was never published when the second row exceeded capacity"
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
}
