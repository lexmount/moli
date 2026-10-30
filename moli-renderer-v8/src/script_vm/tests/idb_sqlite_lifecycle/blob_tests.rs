use super::*;
use moli_indexeddb::sqlite::IndexKeys;
use moli_indexeddb::{IndexedDbExternalObject, IndexedDbValue};

fn value(vm: &mut ScriptVm, code: &str) -> IndexedDbValue {
    vm.with_default_context_scope(|scope, _| {
        let source = v8::String::new(scope, code).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        let value = crate::script_execution::execute_compiled_script(scope, script).unwrap();
        Ok(crate::context_bootstrap::serialize_indexed_db_value_for_test(scope, value).unwrap())
    })
    .unwrap()
}

#[test]
fn idb_sqlite_blob_decode_preserves_identity_and_ignores_unreferenced_slots() {
    let (mut vm, _loader, _backend) = fixture();
    let saved = value(
        &mut vm,
        "(() => { const blob = new Blob(['body'],{type:'text/plain'}); return [blob,blob,new File(['file'],'saved.txt',{type:'text/plain',lastModified:123})]; })()",
    );
    assert_eq!(saved.external_objects().len(), 2);
    vm.with_default_context_scope(|scope, _| {
        let decoded =
            crate::context_bootstrap::deserialize_indexed_db_value_for_test(scope, &saved)
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
        vm.eval(
            "[decoded[0].size,decoded[0].type,decoded[2].name,decoded[2].lastModified].join('|')"
        )
        .unwrap(),
        "4|text/plain|saved.txt|123"
    );
    // The extra unreferenced slot is deliberately never materialized in the Blob store.
    let mut objects = saved.external_objects().to_vec();
    objects.push(IndexedDbExternalObject::Blob {
        bytes: vec![0; 1024 * 1024],
        mime_type: String::new(),
    });
    let saved = IndexedDbValue::new(saved.wire_bytes().to_vec(), objects);
    vm.with_default_context_scope(|scope, _| {
        assert!(
            crate::context_bootstrap::deserialize_indexed_db_value_for_test(scope, &saved)
                .unwrap()
                .is_some()
        );
        Ok(())
    })
    .unwrap();
}

fn seed_partial_blob_collection(vm: &mut ScriptVm, backend: &SqlBackend, blob_size: usize) {
    let transaction = backend.client.allocate_transaction_handle().unwrap();
    let admission = backend
        .connection()
        .queue_transaction_start(
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
    for id in 1..=2 {
        let stored = value(
            vm,
            &format!(
                "(() => {{ const blob = new Blob([new Uint8Array({blob_size})]); (globalThis.seeds ??= []).push(blob); return {{id:{id},group:'g{id}',blob}}; }})()"
            ),
        );
        backend.execute(Command::Write {
            transaction,
            store: "records".into(),
            key: Some(moli_indexeddb::Key::number(id as f64).unwrap()),
            value: if id == 1 {
                stored
            } else {
                vec![1, 2, 3].into()
            },
            indexes: vec![IndexKeys {
                name: "unique".into(),
                keys: vec![moli_indexeddb::Key::from(format!("g{id}"))],
            }],
            add_only: false,
            quota: None,
        });
    }
    backend.execute(Command::Commit {
        transaction,
        quota: None,
    });
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
}

#[tokio::test]
async fn idb_sqlite_partial_blob_collection_failure_releases_earlier_rows_without_gc() {
    let (mut vm, loader, backend) = fixture();
    seed_partial_blob_collection(&mut vm, &backend, 14336);
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
        r#"["error:Error","complete"]"#
    );
    assert_eq!(
        crate::blob::live_blob_count_for_test(),
        before,
        "the successfully decoded first row was never published when the second row contained malformed wire"
    );
}

#[tokio::test]
async fn idb_sqlite_large_blob_and_file_round_trips() {
    for mib in [1, 16, 64] {
        let (mut vm, loader, backend) = fixture();
        let bytes = mib * 1024 * 1024;
        let start = std::time::Instant::now();
        vm.eval(&format!(r#"
            globalThis.largeResult = {{done:false}};
            (async () => {{
                const result = request => new Promise((resolve,reject) => {{
                    request.onsuccess = () => resolve(request.result);
                    request.onerror = () => reject(request.error);
                }});
                const complete = tx => new Promise((resolve,reject) => {{
                    tx.oncomplete = resolve;
                    tx.onabort = () => reject(tx.error || Error('aborted'));
                }});
                const kept = [];
                const keys = [];
                for (const kind of ['blob','file']) {{
                    const input = new Uint8Array({bytes}).fill(91);
                    const payload = kind === 'blob'
                        ? new Blob([input], {{type:'application/octet-stream'}})
                        : new File([input], 'saved.bin', {{type:'application/octet-stream',lastModified:123}});
                    const value = {{group:kind, payload}};
                    const write = db.transaction('records','readwrite');
                    const written = complete(write);
                    const request = write.objectStore('records').put(value);
                    // The request keeps its original snapshot and index key.
                    value.group = 'mutated';
                    input.fill(0);
                    const key = await result(request);
                    await written;
                    const read = db.transaction('records');
                    const readDone = complete(read);
                    const saved = await result(read.objectStore('records').index('unique').get(kind));
                    await readDone;
                    if (saved.id !== key || saved.group !== kind || saved.payload.size !== {bytes})
                        throw Error('large value or index snapshot mismatch');
                    if (kind === 'file' && (!(saved.payload instanceof File)
                        || saved.payload.name !== 'saved.bin' || saved.payload.lastModified !== 123))
                        throw Error('large File metadata mismatch');
                    const erase = db.transaction('records','readwrite');
                    const erased = complete(erase);
                    erase.objectStore('records').delete(key);
                    await erased;
                    kept.push(saved.payload);
                    keys.push(key);
                }}
                const check = db.transaction('records');
                const checked = complete(check);
                const count = await result(check.objectStore('records').count());
                await checked;
                db.close();
                for (const payload of kept) {{
                    const body = new Uint8Array(await payload.arrayBuffer());
                    if (body.length !== {bytes} || body[0] !== 91
                        || body[body.length >> 1] !== 91 || body[body.length-1] !== 91
                        || payload.type !== 'application/octet-stream')
                        throw Error('delivered large bytes did not survive deletion and close');
                }}
                return [keys,count,kept.map(value => value.size)];
            }})().then(value => largeResult = {{done:true,value}},
                error => largeResult = {{done:true,error:String(error),stack:error?.stack}});
        "#)).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(largeResult.done)",
            "true",
            "large SQL Blob/File round trip",
        )
        .await;
        assert_eq!(
            vm.eval("largeResult.error || ''").unwrap(),
            "",
            "{}",
            vm.eval("JSON.stringify(largeResult)").unwrap()
        );
        assert_eq!(
            vm.eval("JSON.stringify(largeResult.value)").unwrap(),
            format!("[[1,2],0,[{bytes},{bytes}]]")
        );
        backend.drain(&mut vm, &loader).await;

        eprintln!(
            "large_blob_file_mib={mib} elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

#[tokio::test]
async fn idb_sqlite_queued_write_snapshots_can_exceed_64_mib() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records', 'readwrite');
        const store = tx.objectStore('records');
        for (let i = 1; i <= 3; i++) {
            const payload = new Uint8Array(24 * 1024 * 1024).fill(i);
            store.put({group:'queued'+i, payload}).onsuccess = event => events.push(event.target.result);
            payload.fill(0);
        }
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('abort:' + tx.error.name);
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[1,2,3,"complete"]"#
    );
    vm.eval(
        r#"(() => {
        const tx = db.transaction('records');
        const store = tx.objectStore('records');
        for (let key = 1; key <= 3; key++) {
            store.get(key).onsuccess = event => {
                const value = event.target.result;
                events.push([value.id, value.group, value.payload.length,
                    value.payload[0], value.payload[value.payload.length-1]]);
            };
        }
        return 'queued';
    })()"#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events.slice(4))").unwrap(),
        "[[1,\"queued1\",25165824,1,1],[2,\"queued2\",25165824,2,2],[3,\"queued3\",25165824,3,3]]"
    );
}

#[tokio::test]
async fn idb_sqlite_partial_blob_collection_failure_releases_previous_batches_without_gc() {
    let (mut vm, loader, backend) = fixture();
    seed_partial_blob_collection(&mut vm, &backend, 2 * 1024 * 1024);
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
        r#"["error:Error","complete"]"#
    );
    assert_eq!(
        crate::blob::live_blob_count_for_test(),
        before,
        "the successfully decoded first row was never published when the second row contained malformed wire"
    );
}
