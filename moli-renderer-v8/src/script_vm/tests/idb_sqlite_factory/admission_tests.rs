use super::*;

#[tokio::test]
async fn idb_sqlite_bulk_requests_progress_on_one_worker_without_task_limits() {
    let config = ServiceConfig::default();

    let manager = crate::new_sqlite_indexed_db_manager(None, config).unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(
        &mut page,
        &loader,
        r#"
        const databases = await Promise.all(Array.from({length:4}, (_, i) =>
            open('bulk-' + i, 1, db => {
                const store = db.createObjectStore('records');
                for (let key = 0; key < 300; ++key) store.put('value-' + key, key);
            })
        ));
        const counts = await Promise.all(databases.map(async db => {
            const transaction = db.transaction('records', 'readwrite');
            const aborted = new Promise(resolve => transaction.onabort = resolve);
            for (let key = 0; key < 300; ++key) transaction.objectStore('records').delete(key);
            transaction.abort();
            await aborted;
            const count = await result(db.transaction('records').objectStore('records').count());
            db.close();
            return count;
        }));
        return counts;
    "#,
    )
    .await;
    assert_eq!(output, "[300,300,300,300]");
}

#[tokio::test]
async fn idb_sqlite_retained_cursors_do_not_block_later_requests_until_gc() {
    let config = ServiceConfig::default();

    let manager = crate::new_sqlite_indexed_db_manager(None, config).unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const db = await open('cursors', 1, db => db.createObjectStore('records').put({value:'retained'}, 1));
        const transaction = db.transaction('records');
        const completed = complete(transaction);
        const store = transaction.objectStore('records');
        // Retain every cursor explicitly: progress must not depend on GC.
        const cursors = await Promise.all(Array.from({length:300}, () => result(store.openCursor())));
        await completed;
        if (cursors.some(cursor => cursor.key !== 1 || cursor.value.value !== 'retained'))
            throw Error('a retained lazy page lost its key or value');
        const count = await result(db.transaction('records').objectStore('records').count());
        db.close();
        return [cursors.length, count];
    "#).await;
    assert_eq!(output, "[300,1]");
}
