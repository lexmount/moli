use super::*;

#[tokio::test]
async fn opfs_source_write_prepares_input_without_blocking_or_reordering_commands() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    run(
        &mut page,
        &loader,
        r#"
        globalThis.root = await navigator.storage.getDirectory();
        globalThis.file = await root.getFileHandle('ordered', {create:true});
        const seed = await file.createWritable(); await seed.write('original'); await seed.close();
        globalThis.writer = await file.createWritable({keepExistingData:true});
        return true;
    "#,
    )
    .await;
    page.eval(r#"
        globalThis.events = [];
        writer.write({type:'write',position:2,data:controlledBlob.slice(100,108)}).then(() => events.push('blob'));
        writer.seek(0).then(() => events.push('seek'));
        writer.write('X').then(() => events.push('text'));
        writer.truncate(10).then(() => events.push('truncate'));
        writer.close().then(() => events.push('close'));
    "#).unwrap();
    assert_eq!(page.eval("events.join('|')").unwrap(), "");
    assert_eq!(source.pending.lock().len(), 1);
    assert_eq!(
        run(
            &mut page,
            &loader,
            "return await (await file.getFile()).text();"
        )
        .await,
        r#""original""#,
        "pending input must not publish the temporary file"
    );
    source.finish(100..108);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "events.join('|')",
        "blob|seek|text|truncate|close",
        "OPFS source FIFO",
    )
    .await;
    assert_eq!(
        run(
            &mut page,
            &loader,
            "return await (await file.getFile()).text();"
        )
        .await,
        r#""Xrdefghijk""#,
    );
}

#[tokio::test]
async fn opfs_source_failures_abort_the_temporary_writer_before_reopening() {
    for missing in [false, true] {
        let manager = manager(None);
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut page = page(&manager, &loader);
        let source = ControlledSource::install(&mut page);
        run(&mut page, &loader, r#"
            globalThis.file = await (await navigator.storage.getDirectory()).getFileHandle('failure',{create:true});
            const seed = await file.createWritable();await seed.write('committed');await seed.close();
            globalThis.writer = await file.createWritable({mode:'exclusive'});
            return true;
        "#).await;
        page.eval(
            r#"
            globalThis.outcome = 'pending';
            writer.write(controlledBlob.slice(100,108)).then(
                () => outcome='unexpected success',
                async error => {
                    const before = await (await file.getFile()).text();
                    const next = await file.createWritable({mode:'exclusive'});
                    await next.write('replacement');await next.close();
                    outcome = error.name+'|'+before;
                });
        "#,
        )
        .unwrap();
        assert_eq!(page.eval("outcome").unwrap(), "pending");
        let mut pending = std::mem::take(&mut *source.pending.lock());
        assert_eq!(pending.len(), 1);
        let (range, complete) = pending.pop().unwrap();
        assert_eq!(range, 100..108);
        complete(Err(io::Error::new(
            if missing {
                io::ErrorKind::NotFound
            } else {
                io::ErrorKind::Other
            },
            "injected OPFS source read failure",
        )));
        let expected = if missing {
            "NotFoundError|committed"
        } else {
            "NotReadableError|committed"
        };
        advance_page_task_executor_until_eval_equals(
            &mut page,
            &loader,
            "outcome",
            expected,
            "OPFS source failure cleanup",
        )
        .await;
        assert_eq!(
            run(
                &mut page,
                &loader,
                "return await (await file.getFile()).text();"
            )
            .await,
            r#""replacement""#,
        );
    }
}

#[tokio::test]
async fn opfs_source_preparation_and_quota_wait_do_not_block_another_bucket() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    run(
        &mut page,
        &loader,
        r#"
        const root = await navigator.storage.getDirectory();
        globalThis.fileA = await root.getFileHandle('a',{create:true});
        globalThis.writerA = await fileA.createWritable();
        const bucketB = await navigator.storageBuckets.open('independent');
        const rootB = await bucketB.getDirectory();
        globalThis.fileB = await rootB.getFileHandle('b',{create:true});
        globalThis.writerB = await fileB.createWritable();
        return true;
    "#,
    )
    .await;
    let buckets = page._context_host.borrow().storage_bucket_store();
    let quota_owner = moli_storage_service::storage_bucket_quota_owner(
        &buckets,
        &moli_storage_service::StorageBucketLocator::default_bucket(storage_key()),
    )
    .unwrap();
    let reservation = quota_owner.reserve_commit();
    page.eval(
        r#"
        globalThis.writtenA = 'pending';
        writerA.write(controlledBlob.slice(100,108)).then(
            () => writtenA='done', error => writtenA=error.name);
    "#,
    )
    .unwrap();
    assert_eq!(
        source.pending.lock().len(),
        1,
        "source reading precedes quota acquisition"
    );
    assert_eq!(
        run(
            &mut page,
            &loader,
            "await writerB.write('while reading');return true;"
        )
        .await,
        "true",
    );
    source.finish(100..108);
    // This also drives the earlier ready input completion into the held
    // bucket's quota queue before the independent write completes.
    assert_eq!(
        run(&mut page, &loader, "await writerB.write(' while waiting');await writerB.close();return await (await fileB.getFile()).text();").await,
        r#""while reading while waiting""#,
    );
    assert_eq!(page.eval("writtenA").unwrap(), "pending");
    drop(reservation);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "writtenA",
        "done",
        "OPFS source quota release",
    )
    .await;
    assert_eq!(
        run(
            &mut page,
            &loader,
            "await writerA.close();return await (await fileA.getFile()).text();"
        )
        .await,
        r#""defghijk""#,
    );
}

#[tokio::test]
async fn opfs_source_completion_after_detach_cannot_write_and_does_not_keep_the_lock() {
    for waiting_on_quota in [false, true] {
        let manager = manager(None);
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut page = page(&manager, &loader);
        let source = ControlledSource::install(&mut page);
        run(
        &mut page,
        &loader,
        r#"
        globalThis.frame = document.createElement('iframe');document.body.append(frame);
        const child = frame.contentWindow;
        const root = await child.navigator.storage.getDirectory();
        const file = await root.getFileHandle('detached',{create:true});
        const seed = await file.createWritable();await seed.write('committed');await seed.close();
        child.writer = await file.createWritable({mode:'exclusive'});
        globalThis.parentFile = await (await navigator.storage.getDirectory()).getFileHandle('detached');
        return true;
    "#,
    )
    .await;
        let reservation = waiting_on_quota.then(|| {
            let buckets = page._context_host.borrow().storage_bucket_store();
            moli_storage_service::storage_bucket_quota_owner(
                &buckets,
                &moli_storage_service::StorageBucketLocator::default_bucket(storage_key()),
            )
            .unwrap()
            .reserve_commit()
        });
        page.eval(
            r#"
        globalThis.outcome = 'pending';
        frame.contentWindow.writer.write(controlledBlob.slice(100,108)).then(
            () => outcome='success', error => outcome=error.name);
    "#,
        )
        .unwrap();
        assert_eq!(source.pending.lock().len(), 1);
        if waiting_on_quota {
            source.finish(100..108);
            // The input completion is older than this metadata result. Drive it
            // into quota wait without releasing the held bucket reservation.
            assert_eq!(
                run(
                    &mut page,
                    &loader,
                    "await navigator.storage.getDirectory();return true;"
                )
                .await,
                "true"
            );
            assert_eq!(page.eval("outcome").unwrap(), "pending");
            assert!(page.has_pending_opfs_tasks());
        }
        page.eval("frame.remove(); 'detached'").unwrap();
        assert_eq!(
            run(
                &mut page,
                &loader,
                r#"
        const before = await (await parentFile.getFile()).text();
        globalThis.nextWriter = await parentFile.createWritable({mode:'exclusive'});
        return before;
    "#
            )
            .await,
            r#""committed""#,
            "retiring the realm must release its writer without waiting for input or quota"
        );
        drop(reservation);
        run(
            &mut page,
            &loader,
            "await nextWriter.write('replacement');await nextWriter.close();return true;",
        )
        .await;
        if !waiting_on_quota {
            source.finish(100..108);
        }
        assert_eq!(
            run(
                &mut page,
                &loader,
                r#"
        return await (await parentFile.getFile()).text();
    "#
            )
            .await,
            r#""replacement""#
        );
        assert_eq!(page.eval("outcome").unwrap(), "pending");
    }
}

#[tokio::test]
async fn opfs_source_completion_cannot_target_a_replacement_storage_service() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    run(&mut page, &loader, r#"
        const file=await (await navigator.storage.getDirectory()).getFileHandle('old',{create:true});
        globalThis.oldWriter=await file.createWritable({mode:'exclusive'});
        return true;
    "#).await;
    page.eval(
        r#"
        globalThis.oldOutcome='pending';
        oldWriter.write(controlledBlob.slice(100,108)).then(
            ()=>oldOutcome='unexpected success',error=>oldOutcome=error.name);
    "#,
    )
    .unwrap();
    assert_eq!(source.pending.lock().len(), 1);
    page.set_storage_bucket_store(crate::new_shared_storage_bucket_store());
    // Both services allocate their first writer id. A late completion must
    // neither write to nor abort the replacement service's unrelated writer.
    run(&mut page, &loader, r#"
        globalThis.newFile=await (await navigator.storage.getDirectory()).getFileHandle('new',{create:true});
        globalThis.newWriter=await newFile.createWritable({mode:'exclusive'});
        await newWriter.write('replacement');return true;
    "#).await;
    source.finish(100..108);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "oldOutcome",
        "SecurityError",
        "OPFS input service identity",
    )
    .await;
    assert_eq!(
        run(
            &mut page,
            &loader,
            "await newWriter.close();return await (await newFile.getFile()).text();"
        )
        .await,
        r#""replacement""#,
    );
}

#[tokio::test]
async fn opfs_writes_a_committed_sqlite_blob_after_database_close_in_window_and_worker() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    assert_eq!(run(&mut page, &loader, r#"
        const db=await open('opfs-source',1,db=>db.createObjectStore('records'));
        const write=db.transaction('records','readwrite');
        write.objectStore('records').put(new Blob(['from sqlite']),1);await complete(write);
        const read=db.transaction('records');
        globalThis.savedBlob=await result(read.objectStore('records').get(1));await complete(read);db.close();
        return savedBlob.size;
    "#).await, "11");
    page.with_default_context_scope(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let object = global
            .get(scope, crate::util::v8str(scope, "savedBlob").into())
            .unwrap();
        let object = v8::Local::<v8::Object>::try_from(object).unwrap();
        assert!(
            crate::blob::blob_data_from_object(scope, object)
                .unwrap()
                .as_bytes()
                .is_none()
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(run(&mut page, &loader, r#"
        const file=await (await navigator.storage.getDirectory()).getFileHandle('sqlite',{create:true});
        const writer=await file.createWritable();await writer.write(savedBlob);await writer.close();
        const code=`onmessage=async()=>{try{
            const db=await new Promise((resolve,reject)=>{const r=indexedDB.open('opfs-source');r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
            const blob=await new Promise((resolve,reject)=>{const r=db.transaction('records').objectStore('records').get(1);r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});db.close();
            const file=await (await navigator.storage.getDirectory()).getFileHandle('worker-sqlite',{create:true});
            const writer=await file.createWritable();await writer.write(blob);await writer.close();
            postMessage(await (await file.getFile()).text());close();
        }catch(error){postMessage(error.name+':'+error.message);close();}}`;
        const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
        const worker=new Worker(url);
        const answer=new Promise((resolve,reject)=>{worker.onmessage=e=>resolve(e.data);worker.onerror=reject;});
        worker.postMessage('start');const workerText=await answer;worker.terminate();URL.revokeObjectURL(url);
        await result(indexedDB.deleteDatabase('opfs-source'));
        return [await (await file.getFile()).text(),workerText];
    "#).await, r#"["from sqlite","from sqlite"]"#);
}
