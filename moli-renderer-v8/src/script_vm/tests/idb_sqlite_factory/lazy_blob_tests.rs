use super::*;
use moli_file_api::{BlobData, BlobReadCompletion, BlobSource};
use std::{fmt, io, ops::Range, sync::Arc};

mod body_tests;
mod clipboard_tests;
mod opfs_tests;
mod websocket_tests;

struct ControlledSource {
    bytes: Vec<u8>,
    pending: parking_lot::Mutex<Vec<(Range<usize>, BlobReadCompletion)>>,
    requested: tokio::sync::Notify,
}

impl fmt::Debug for ControlledSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ControlledSource")
    }
}

impl BlobSource for ControlledSource {
    fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
        self.pending.lock().push((range, complete));
        self.requested.notify_one();
    }
}

impl ControlledSource {
    fn install(page: &mut crate::runtime::PageVmTaskExecutorTestHarness) -> Arc<Self> {
        let source = Arc::new(Self {
            bytes: (0..200_000).map(|index| (index % 251) as u8).collect(),
            pending: parking_lot::Mutex::new(Vec::new()),
            requested: tokio::sync::Notify::new(),
        });
        let data = BlobData::from_source(source.clone(), source.bytes.len());
        page.with_default_context_scope(|scope, _| {
            let object = crate::blob::build_blob_object_from_data(
                scope,
                data,
                "application/octet-stream".into(),
            )
            .unwrap();
            let name = crate::util::v8str(scope, "controlledBlob");
            assert_eq!(
                scope
                    .get_current_context()
                    .global(scope)
                    .set(scope, name.into(), object.into()),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
        source
    }

    fn finish(&self, expected: Range<usize>) {
        let pending = std::mem::take(&mut *self.pending.lock());
        assert_eq!(pending.len(), 1);
        let (range, complete) = pending.into_iter().next().unwrap();
        assert_eq!(range, expected);
        complete(Ok(self.bytes[range].to_vec()));
    }
}

#[tokio::test]
async fn blob_sources_stay_lazy_through_metadata_clone_composition_and_stream_creation() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    assert_eq!(page.eval(r#"(() => {
        globalThis.clone = structuredClone(controlledBlob);
        globalThis.slice = clone.slice(100, 108);
        globalThis.composed = new Blob([slice,'tail']);
        globalThis.file = new File([composed], 'kept.bin', {lastModified:123});
        globalThis.form = new FormData();
        form.append('blob', clone);
        form.append('file', file);
        form.set('alias', file, 'alias.bin');
        globalThis.aliasFile = form.get('alias');
        globalThis.stream = controlledBlob.stream();
        return [controlledBlob.size,slice.size,composed.size,file.size,file.name,file.lastModified,
            form.get('file') === file,form.get('blob').name,aliasFile.name,aliasFile.size,aliasFile.lastModified].join('|');
    })()"#).unwrap(), "200000|8|12|12|kept.bin|123|true|blob|alias.bin|12|123");
    page.with_default_context_scope(|scope, _| {
        let key = crate::util::v8str(scope, "aliasFile");
        let file = scope
            .get_current_context()
            .global(scope)
            .get(scope, key.into())
            .unwrap();
        let file = v8::Global::new(scope, file);
        assert_eq!(
            crate::context_bootstrap::form_data_entries_to_string_pairs(
                scope,
                &[("upload".to_owned(), file)],
            ),
            [("upload".to_owned(), "alias.bin".to_owned())],
        );
        Ok(())
    })
    .unwrap();
    assert!(
        source.pending.lock().is_empty(),
        "metadata and stream construction perform no read"
    );
    page.eval("globalThis.readResult = 'pending'; aliasFile.bytes().then(bytes => readResult = Array.from(bytes).join(',')); 'queued'").unwrap();
    assert_eq!(page.eval("readResult").unwrap(), "pending");
    source.finish(100..108);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "readResult",
        "100,101,102,103,104,105,106,107,116,97,105,108",
        "lazy Blob file read",
    )
    .await;
    assert!(source.pending.lock().is_empty());
    page.eval("globalThis.reader = stream.getReader(); globalThis.firstChunk = 'pending'; reader.read().then(value => firstChunk = String(value.value.length)); 'queued'").unwrap();
    assert_eq!(page.eval("firstChunk").unwrap(), "pending");
    source.finish(0..65536);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "firstChunk",
        "65536",
        "lazy Blob stream pull",
    )
    .await;
    assert!(
        source.pending.lock().is_empty(),
        "backpressure does not prefetch the next chunk"
    );
    page.eval("reader.cancel(); 'cancelled'").unwrap();
    page.eval(
        r#"(() => {
        const frame = document.createElement('iframe'); document.body.appendChild(frame);
        globalThis.childRealm = frame.contentWindow;
        globalThis.foreignBytes = 'pending';
        childRealm.Blob.prototype.bytes.call(controlledBlob.slice(100,108)).then(value => {
            foreignBytes = String(Object.getPrototypeOf(value) === Uint8Array.prototype);
        });
        return 'queued';
    })()"#,
    )
    .unwrap();
    source.finish(100..108);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "foreignBytes",
        "true",
        "Blob completion in the Blob realm despite a borrowed method",
    )
    .await;
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}

#[tokio::test]
async fn lazy_blob_file_reader_waits_for_bytes_and_preserves_each_read_mode() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    for (method, extra, expected) in [
        (
            "readAsText",
            ", 'utf-16le'",
            "\u{eae9}\u{eceb}\u{eeed}\u{f0ef}",
        ),
        ("readAsBinaryString", "", "éêëìíîïð"),
        ("readAsDataURL", "", "data:text/plain;base64,6err7O3u7/A="),
        ("readAsArrayBuffer", "", "233,234,235,236,237,238,239,240"),
    ] {
        let start = page.eval(&format!(r#"(() => {{
            globalThis.reader = new FileReader();
            globalThis.events = [];
            globalThis.readResult = 'pending';
            for (const type of ['loadstart','progress','load','error','abort','loadend']) {{
                reader.addEventListener(type, () => events.push(type));
            }}
            reader.onloadend = () => {{
                readResult = reader.result instanceof ArrayBuffer
                    ? Array.from(new Uint8Array(reader.result)).join(',') : String(reader.result);
            }};
            reader.{method}(controlledBlob.slice(233,241,'text/plain'){extra});
            return [reader.readyState,reader.result,reader.error,events.length].map(String).join('|');
        }})()"#)).unwrap();
        assert_eq!(
            start, "1|null|null|0",
            "{method} must return before I/O completes"
        );
        assert_eq!(source.pending.lock().len(), 1);
        advance_page_task_executor_until_eval_equals(
            &mut page,
            &loader,
            "events.join('|')",
            "loadstart",
            "FileReader loadstart before body completion",
        )
        .await;
        assert_eq!(page.eval("String(reader.readyState)").unwrap(), "1");
        source.finish(233..241);
        advance_page_task_executor_until_eval_equals(
            &mut page,
            &loader,
            "String(readResult !== 'pending')",
            "true",
            "source FileReader result",
        )
        .await;
        assert_eq!(page.eval("readResult").unwrap(), expected, "{method}");
        assert_eq!(
            page.eval("[reader.readyState,reader.error,events.join(',')].map(String).join('|')")
                .unwrap(),
            "2|null|loadstart,progress,load,loadend",
            "{method}"
        );
    }
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}

#[tokio::test]
async fn lazy_blob_file_reader_io_failure_is_an_async_not_readable_error() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    assert_eq!(
        page.eval(
            r#"(() => {
        globalThis.reader = new FileReader();
        globalThis.events = [];
        for (const type of ['loadstart','progress','load','error','loadend']) {
            reader.addEventListener(type, () => events.push(type));
        }
        reader.readAsArrayBuffer(controlledBlob.slice(0,16));
        return [reader.readyState,reader.result,reader.error].map(String).join('|');
    })()"#
        )
        .unwrap(),
        "1|null|null"
    );
    let (range, complete) = source.pending.lock().pop().unwrap();
    assert_eq!(range, 0..16);
    complete(Err(io::Error::other("read failed")));
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "events.join('|')",
        "loadstart|error|loadend",
        "FileReader asynchronous read error",
    )
    .await;
    assert_eq!(
        page.eval("[reader.readyState,reader.result,reader.error.name].map(String).join('|')")
            .unwrap(),
        "2|null|NotReadableError"
    );
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}

#[tokio::test]
async fn lazy_blob_file_reader_abort_discards_an_old_completion_after_reuse() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    page.eval(
        r#"(() => {
        globalThis.reader = new FileReader();
        globalThis.events = [];
        globalThis.readResult = 'pending';
        for (const type of ['loadstart','progress','load','error','abort','loadend']) {
            reader.addEventListener(type, () => events.push(type));
        }
        reader.onload = () => readResult = Array.from(new Uint8Array(reader.result)).join(',');
        reader.readAsText(controlledBlob.slice(100,108));
        reader.abort();
        reader.readAsArrayBuffer(controlledBlob.slice(200,208));
        return 'queued';
    })()"#,
    )
    .unwrap();
    let mut pending = std::mem::take(&mut *source.pending.lock());
    assert_eq!(pending.len(), 2);
    let (old_range, old_complete) = pending.remove(0);
    let (new_range, new_complete) = pending.remove(0);
    assert_eq!(old_range, 100..108);
    assert_eq!(new_range, 200..208);
    new_complete(Ok(source.bytes[new_range].to_vec()));
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "readResult",
        "200,201,202,203,204,205,206,207",
        "new read after FileReader abort",
    )
    .await;
    let expected_events = page.eval("events.join('|')").unwrap();
    assert_eq!(
        expected_events,
        "abort|loadend|loadstart|progress|load|loadend"
    );
    old_complete(Err(io::Error::other("late aborted read")));
    assert!(
        page.run_one_file_reading_task_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(page.eval("events.join('|')").unwrap(), expected_events);
    assert_eq!(page.eval("[reader.readyState,reader.error,Array.from(new Uint8Array(reader.result)).join(',')].map(String).join('|')").unwrap(),
        "2|null|200,201,202,203,204,205,206,207");
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}

#[tokio::test]
async fn lazy_blob_late_completion_after_page_retirement_does_not_enter_a_new_realm() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    page.eval("controlledBlob.arrayBuffer().then(() => { throw Error('retired Blob callback'); }); 'queued'").unwrap();
    assert_eq!(source.pending.lock().len(), 1);
    drop(page);
    source.finish(0..200_000);
    let mut replacement = super::page(&manager, &loader);
    assert_eq!(
        replacement.eval("typeof controlledBlob").unwrap(),
        "undefined"
    );
    drop(replacement);
    manager.lock().shutdown_sqlite().unwrap();
}

#[tokio::test]
async fn lazy_blob_read_failure_rejects_and_idb_inline_write_rolls_back_before_the_next_request() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    run(&mut page, &loader, r#"
        globalThis.db = await open('failed-blob', 1, db => db.createObjectStore('records',{keyPath:'id',autoIncrement:true}));
        return db.version;
    "#).await;
    page.eval(
        r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        const bad = store.put({blob:controlledBlob});
        const good = store.put({value:'kept'});
        bad.onerror = event => { event.preventDefault(); events.push('error:'+bad.error.name); };
        good.onsuccess = () => events.push('key:'+good.result);
        tx.oncomplete = () => events.push('complete');
        return 'queued';
    })()"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(events.length)",
        "0",
        "no premature write result",
    )
    .await;
    // PrepareWrite runs before reading the attachment. Drive the accepting
    // Page tasks until that actual native read has been submitted.
    while source.pending.lock().is_empty() {
        wait_for_one_selected_page_task_executor_test_turn(&mut page, &loader)
            .await
            .unwrap();
    }
    let (_, complete) = source.pending.lock().pop().unwrap();
    complete(Err(io::Error::other("injected Blob read error")));
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "JSON.stringify(events)",
        r#"["error:UnknownError","key:1","complete"]"#,
        "Blob write rollback",
    )
    .await;
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}

#[tokio::test]
async fn idb_lazy_blob_and_file_references_survive_copy_delete_and_worker_clone() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const db = await open('lazy-roundtrip', 1, db => db.createObjectStore('records',{keyPath:'id',autoIncrement:true}));
        const input = new Uint8Array(200000);
        for(let i=0;i<input.length;i++) input[i]=i%251;
        const write = db.transaction('records','readwrite');
        write.objectStore('records').put({id:1,blob:new Blob([input]),file:new File([input],'saved.bin',{lastModified:123})});
        await complete(write);
        const read = db.transaction('records');
        const saved = await result(read.objectStore('records').get(1));
        await complete(read);
        const clone = structuredClone(saved);
        const copy = db.transaction('records','readwrite');
        const key = await result(copy.objectStore('records').put({blob:new Blob([clone.blob.slice(100,108),'tail']),file:clone.file}));
        await complete(copy);
        const verify = db.transaction('records');
        const copied = await result(verify.objectStore('records').get(key));
        await complete(verify);
        const workerUrl = URL.createObjectURL(new Blob([`onmessage = async event => {
            const blob = event.data;
            const bytes = await blob.slice(100,108).bytes();
            postMessage([blob.size,Array.from(bytes)]);
            close();
        }`],{type:'text/javascript'}));
        const worker = new Worker(workerUrl);
        const workerResult = new Promise((resolve,reject) => {worker.onmessage=event=>resolve(event.data);worker.onerror=reject;});
        worker.postMessage(clone.file);
        db.close();
        await result(indexedDB.deleteDatabase('lazy-roundtrip'));
        const bytes = await copied.blob.bytes();
        const tail = await clone.file.slice(199992).bytes();
        const workerValue = await workerResult;
        worker.terminate(); URL.revokeObjectURL(workerUrl);
        return [key,Array.from(bytes),Array.from(tail),clone.file.name,clone.file.lastModified,workerValue];
    "#).await;
    assert_eq!(
        output,
        r#"[2,[100,101,102,103,104,105,106,107,116,97,105,108],[196,197,198,199,200,201,202,203],"saved.bin",123,[200000,[100,101,102,103,104,105,106,107]]]"#
    );
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}
