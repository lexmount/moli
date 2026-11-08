use super::*;

#[tokio::test]
async fn idb_sqlite_quota_rejects_schema_only_upgrade_and_allows_a_later_open() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const bucket = await navigator.storageBuckets.open('schema-limit',{quota:8});
        const events = [];
        await new Promise((resolve,reject) => {
            const request = bucket.indexedDB.open('records',1);
            request.onupgradeneeded = () => {
                request.result.createObjectStore('discarded');
                const tx = request.transaction;
                tx.onabort = () => events.push('abort:' + tx.error.name);
            };
            request.onsuccess = () => reject(Error('schema quota was not checked'));
            request.onerror = event => { event.preventDefault(); events.push('open-error:' + request.error.name); resolve(); };
        });
        if ((await bucket.indexedDB.databases()).length) throw Error('aborted schema was published');
        const raised = await navigator.storageBuckets.open('schema-limit',{quota:4096});
        const db = await new Promise((resolve,reject) => {
            const request = raised.indexedDB.open('records',1);
            request.onupgradeneeded = event => {
                events.push('upgrade:' + event.oldVersion + ':' + request.result.objectStoreNames.length);
                request.result.createObjectStore('kept');
            };
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
        });
        events.push('success:' + db.version + ':' + Array.from(db.objectStoreNames).join(','));
        db.close();
        return events;
    "#).await;
    assert_eq!(
        output,
        r#"["abort:QuotaExceededError","open-error:AbortError","upgrade:0:0","success:1:kept"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_quota_request_error_is_recoverable_and_restores_generated_keys() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page,&loader,r#"
        const bucket = await navigator.storageBuckets.open('limited',{quota:1024});
        const errors = [], keys = [];
        for (const inline of [false,true]) {
            const db = await new Promise((resolve,reject) => {
                const request = bucket.indexedDB.open('records-' + inline,1);
                request.onerror = () => reject(request.error);
                request.onsuccess = () => resolve(request.result);
                request.onupgradeneeded = () => {
                    const store = request.result.createObjectStore('records',inline ? {keyPath:'id',autoIncrement:true} : {autoIncrement:true});
                    if (inline) store.createIndex('tag','tag');
                };
            });
            const tx = db.transaction('records','readwrite');
            const done = complete(tx);
            const store = tx.objectStore('records');
            const bad = store.put(inline ? {tag:'bad',payload:'x'.repeat(4096)} : 'x'.repeat(4096));
            bad.onerror = event => {
                event.preventDefault();
                errors.push([bad.error.name,bad.error.quota === 1024,bad.error.requested > 1024]);
            };
            const good = store.put(inline ? {tag:'kept',payload:'small'} : 'small');
            good.onsuccess = () => keys.push(good.result);
            await done;
            const values = await result(db.transaction('records').objectStore('records').getAll());
            if (values.length !== 1 || (inline && values[0].id !== 1)) throw Error('request rollback failed');
            db.close();
        }
        return {errors,keys};
    "#).await;
    assert_eq!(
        output,
        r#"{"errors":[["QuotaExceededError",true,true],["QuotaExceededError",true,true]],"keys":[1,1]}"#
    );
}

#[tokio::test]
async fn idb_sqlite_quota_commit_rechecks_other_databases_in_the_same_bucket() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page,&loader,r#"
        const bucket = await navigator.storageBuckets.open('shared',{quota:2048});
        const databases = [];
        for (const name of ['first','second']) {
            databases.push(await new Promise((resolve,reject) => {
                const request = bucket.indexedDB.open(name,1);
                request.onupgradeneeded = () => request.result.createObjectStore('records');
                request.onerror = () => reject(request.error);
                request.onsuccess = () => resolve(request.result);
            }));
        }
        let release = false, successful = 0, ready;
        const bothRequests = new Promise(resolve => ready = resolve);
        const outcomes = databases.map(db => new Promise(resolve => {
            const tx = db.transaction('records','readwrite'), store = tx.objectStore('records');
            tx.oncomplete = () => resolve('complete');
            tx.onabort = () => resolve(tx.error.name);
            store.put('x'.repeat(1400),1).onsuccess = () => {
                if (++successful === 2) ready();
                const keepActive = () => { store.get(1).onsuccess = () => { if (!release) keepActive(); }; };
                keepActive();
            };
        }));
        await bothRequests;
        release = true;
        const settled = (await Promise.all(outcomes)).sort();
        let records = 0;
        for (const db of databases) { records += await result(db.transaction('records').objectStore('records').count()); db.close(); }
        return {successful,settled,records};
    "#).await;
    assert_eq!(
        output,
        r#"{"successful":2,"settled":["QuotaExceededError","complete"],"records":1}"#
    );
}

#[tokio::test]
async fn idb_sqlite_quota_commit_rechecks_cache_and_opfs_growth() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page,&loader,r#"
        const outcomes = [];
        for (const backend of ['cache','opfs']) {
            const bucket = await navigator.storageBuckets.open(backend,{quota:2048});
            const db = await new Promise((resolve,reject) => {
                const request = bucket.indexedDB.open('records',1);
                request.onupgradeneeded = () => request.result.createObjectStore('records');
                request.onerror = () => reject(request.error);
                request.onsuccess = () => resolve(request.result);
            });
            let release = false, ready;
            const written = new Promise(resolve => ready = resolve);
            const tx = db.transaction('records','readwrite'), store = tx.objectStore('records');
            const settled = new Promise(resolve => {
                tx.oncomplete = () => resolve('unexpected-complete');
                tx.onabort = () => resolve(tx.error.name);
            });
            store.put('x'.repeat(1400),1).onsuccess = () => {
                ready();
                const keepActive = () => { store.get(1).onsuccess = () => { if (!release) keepActive(); }; };
                keepActive();
            };
            await written;
            if (backend === 'cache') {
                const cache = await bucket.caches.open('cache');
                await cache.put('/reserved',new Response('x'.repeat(1400)));
            } else {
                const root = await bucket.getDirectory(), file = await root.getFileHandle('reserved',{create:true});
                const writer = await file.createWritable();
                await writer.write('x'.repeat(1400));
                await writer.close();
            }
            release = true;
            outcomes.push([backend,await settled,await result(db.transaction('records').objectStore('records').count())]);
            db.close();
        }
        return outcomes;
    "#).await;
    assert_eq!(
        output,
        r#"[["cache","QuotaExceededError",0],["opfs","QuotaExceededError",0]]"#
    );
}

#[tokio::test]
async fn idb_sqlite_abort_settles_quota_waiting_request_before_transaction_event() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    run(&mut page, &loader, r#"
        globalThis.quotaAbortDatabase = await open('quota-abort', 1, db => db.createObjectStore('records'));
        return 'ready';
    "#).await;
    let buckets = page._context_host.borrow().storage_bucket_store();
    let quota_owner = moli_storage_service::storage_bucket_quota_owner(
        &buckets,
        &moli_storage_service::StorageBucketLocator::default_bucket(storage_key()),
    )
    .unwrap();
    let reservation = quota_owner.reserve_commit();
    page.eval(r#"
        globalThis.quotaAbortEvents = [];
        globalThis.quotaAbortTransaction = quotaAbortDatabase.transaction('records', 'readwrite');
        const pending = quotaAbortTransaction.objectStore('records').put('discarded', 1);
        pending.onsuccess = () => quotaAbortEvents.push('unexpected-success');
        pending.onerror = event => { event.preventDefault(); quotaAbortEvents.push('request:' + pending.error.name); };
        quotaAbortTransaction.onabort = () => quotaAbortEvents.push('abort:' + quotaAbortTransaction.error);
        quotaAbortTransaction.oncomplete = () => quotaAbortEvents.push('unexpected-complete');
    "#).unwrap();
    loop {
        let started = page
            .with_default_context_scope(|scope, _| {
                let global = scope.get_current_context().global(scope);
                let transaction = global
                    .get(
                        scope,
                        crate::util::v8str(scope, "quotaAbortTransaction").into(),
                    )
                    .unwrap();
                let transaction = v8::Local::<v8::Object>::try_from(transaction).unwrap();
                Ok(crate::context_bootstrap::sql_transaction_started_for_test(
                    scope,
                    transaction,
                ))
            })
            .unwrap();
        if started {
            break;
        }
        wait_for_one_selected_page_task_executor_test_turn(&mut page, &loader)
            .await
            .unwrap();
    }
    // BEGIN has completed and the write is waiting on the held quota owner.
    // Rollback uses its reserved control slot and must not wait for that write.
    page.eval("quotaAbortTransaction.abort()").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(quotaAbortEvents.some(event => event.startsWith('abort:')))",
        "true",
        "abort while quota is held",
    )
    .await;
    assert_eq!(
        page.eval("JSON.stringify(quotaAbortEvents)").unwrap(),
        r#"["request:AbortError","abort:null"]"#
    );
    drop(reservation);
    let final_state = run(&mut page, &loader, r#"
        const next = quotaAbortDatabase.transaction('records', 'readwrite');
        const done = complete(next);
        next.objectStore('records').put('kept', 2);
        await done;
        const count = await result(quotaAbortDatabase.transaction('records').objectStore('records').count());
        quotaAbortDatabase.close();
        return {events:quotaAbortEvents, count};
    "#).await;
    assert_eq!(
        final_state,
        r#"{"events":["request:AbortError","abort:null"],"count":1}"#
    );
}
