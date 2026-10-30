use super::*;

#[tokio::test]
async fn clipboard_source_representations_are_read_asynchronously_and_published_together() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    let metadata = run(
        &mut page,
        &loader,
        r#"
        await navigator.clipboard.writeText('before');
        globalThis.clipboardItem = new ClipboardItem({
            'text/plain': new Blob([controlledBlob.slice(100,108)], {type:'text/plain'}),
            'text/html': new Blob([controlledBlob.slice(108,116)], {type:'text/html'})
        }, {presentationStyle:'attachment'});
        const blob = await clipboardItem.getType('text/plain');
        return [blob.size,blob.type];
    "#,
    )
    .await;
    assert_eq!(metadata, r#"[8,"text/plain"]"#);
    assert!(
        source.pending.lock().is_empty(),
        "constructing an item and getType perform no I/O"
    );
    page.eval(
        r#"
        globalThis.clipboardWrite = 'pending';
        navigator.clipboard.write([clipboardItem]).then(
            () => clipboardWrite = 'done', error => clipboardWrite = error.name);
    "#,
    )
    .unwrap();
    assert_eq!(page.eval("clipboardWrite").unwrap(), "pending");
    let mut pending = std::mem::take(&mut *source.pending.lock());
    assert_eq!(pending.len(), 2);
    let index = pending
        .iter()
        .position(|(range, _)| *range == (108..116))
        .unwrap();
    let (range, complete_html) = pending.remove(index);
    complete_html(Ok(source.bytes[range].to_vec()));
    assert!(
        page.run_one_file_reading_task_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(page.eval("clipboardWrite").unwrap(), "pending");
    assert_eq!(
        run(
            &mut page,
            &loader,
            "return await navigator.clipboard.readText();"
        )
        .await,
        "\"before\""
    );
    let (range, complete_plain) = pending.pop().unwrap();
    assert_eq!(range, 100..108);
    complete_plain(Ok(source.bytes[range].to_vec()));
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "clipboardWrite",
        "done",
        "source clipboard write",
    )
    .await;
    assert_eq!(
        run(
            &mut page,
            &loader,
            r#"
        const [item] = await navigator.clipboard.read();
        const text = await navigator.clipboard.readText();
        const html = await (await item.getType('text/html')).text();
        return [text,html,item.types.join('|'),item.presentationStyle];
    "#
        )
        .await,
        r#"["defghijk","lmnopqrs","text/plain|text/html","attachment"]"#
    );
    assert!(source.pending.lock().is_empty());
}

#[tokio::test]
async fn clipboard_source_read_failure_is_data_error_without_replacing_the_snapshot() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    run(&mut page, &loader, r#"
        await navigator.clipboard.write([new ClipboardItem({'text/plain':'before'}, {presentationStyle:'inline'})]);
        return true;
    "#).await;
    page.eval(
        r#"
        globalThis.clipboardWrite = 'pending';
        navigator.clipboard.write([new ClipboardItem({
            'text/plain': new Blob([controlledBlob.slice(100,108)], {type:'text/plain'})
        }, {presentationStyle:'attachment'})]).then(
            () => clipboardWrite = 'unexpected success',
            error => clipboardWrite = error.name);
    "#,
    )
    .unwrap();
    assert_eq!(page.eval("clipboardWrite").unwrap(), "pending");
    let pending = std::mem::take(&mut *source.pending.lock());
    assert_eq!(pending.len(), 1);
    let (range, complete) = pending.into_iter().next().unwrap();
    assert_eq!(range, 100..108);
    complete(Err(io::Error::other(
        "injected clipboard body read failure",
    )));
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "clipboardWrite",
        "DataError",
        "failed source clipboard write",
    )
    .await;
    assert_eq!(run(&mut page, &loader, r#"
        const marker = new DOMException('author failure','NotReadableError');
        let caught;
        try {await navigator.clipboard.write([new ClipboardItem({'text/plain':Promise.reject(marker)})]);}
        catch(error) {caught=error;}
        return [caught===marker,caught.name,await navigator.clipboard.readText(),(await navigator.clipboard.read())[0].presentationStyle];
    "#).await, r#"[true,"NotReadableError","before","inline"]"#);
}

#[tokio::test]
async fn clipboard_source_completion_after_receiver_detach_does_not_publish_or_settle() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    run(
        &mut page,
        &loader,
        "await navigator.clipboard.writeText('before'); return true;",
    )
    .await;
    page.eval(
        r#"
        globalThis.clipboardWrite = 'pending';
        globalThis.clipboardFrame = document.createElement('iframe');
        document.body.append(clipboardFrame);
        const child = clipboardFrame.contentWindow;
        child.navigator.clipboard.write([new child.ClipboardItem({
            'text/plain': new child.Blob([controlledBlob.slice(100,108)], {type:'text/plain'})
        })]).then(() => clipboardWrite = 'success', error => clipboardWrite = error.name);
    "#,
    )
    .unwrap();
    assert_eq!(page.eval("clipboardWrite").unwrap(), "pending");
    assert_eq!(source.pending.lock().len(), 1);
    page.eval("clipboardFrame.remove(); 'detached'").unwrap();
    source.finish(100..108);
    page.run_one_file_reading_task_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(
        run(
            &mut page,
            &loader,
            "return await navigator.clipboard.readText();"
        )
        .await,
        "\"before\""
    );
    assert_eq!(page.eval("clipboardWrite").unwrap(), "pending");
}

#[tokio::test]
async fn clipboard_reads_a_committed_sqlite_blob_and_keeps_its_snapshot_after_database_deletion() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    assert_eq!(
        run(
            &mut page,
            &loader,
            r#"
        const db = await open('clipboard-source',1,db => db.createObjectStore('records'));
        const write = db.transaction('records','readwrite');
        write.objectStore('records').put(new Blob(['clipboard from sqlite'],{type:'text/plain'}),1);
        await complete(write);
        const read = db.transaction('records');
        globalThis.savedClipboardBlob = await result(read.objectStore('records').get(1));
        await complete(read);
        db.close();
        return savedClipboardBlob.size;
    "#
        )
        .await,
        "21"
    );
    page.with_default_context_scope(|scope, _| {
        let key = crate::util::v8str(scope, "savedClipboardBlob");
        let object = scope
            .get_current_context()
            .global(scope)
            .get(scope, key.into())
            .unwrap();
        let object = v8::Local::<v8::Object>::try_from(object).unwrap();
        let data = crate::blob::blob_data_from_object(scope, object).unwrap();
        assert!(
            data.as_bytes().is_none(),
            "the read must exercise an actual committed SQLite source"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(run(&mut page, &loader, r#"
        await navigator.clipboard.write([new ClipboardItem({'text/plain':savedClipboardBlob})]);
        await result(indexedDB.deleteDatabase('clipboard-source'));
        const [item] = await navigator.clipboard.read();
        return [await navigator.clipboard.readText(),await (await item.getType('text/plain')).text()];
    "#).await, r#"["clipboard from sqlite","clipboard from sqlite"]"#);
}
