use super::*;

#[tokio::test]
async fn idb_sqlite_bucket_delete_clears_storage_and_invalidates_retained_objects() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let identity = page
        .storage_bucket_store
        .lock()
        .open_bucket(&storage_key(), "remove")
        .unwrap();
    let output = run(&mut page, &loader, r#"
        const bucket = await navigator.storageBuckets.open('remove');
        const factory = bucket.indexedDB;
        const db = await new Promise((resolve,reject) => {
            const request = factory.open('records',1);
            request.onupgradeneeded = () => request.result.createObjectStore('records',{keyPath:'id',autoIncrement:true});
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
        });
        const seed = db.transaction('records','readwrite'), seeded = complete(seed);
        seed.objectStore('records').put({blob:new Blob(['kept after deletion'])});
        await seeded;
        const saved = await result(db.transaction('records').objectStore('records').get(1));
        const tx = db.transaction('records','readwrite');
        const outcome = new Promise(resolve => {
            tx.onabort = () => resolve('abort');
            tx.oncomplete = () => resolve('unexpected-commit');
        });
        tx.objectStore('records').put({blob:new Blob(['discarded'])}).onerror = event => event.preventDefault();
        await Promise.all([navigator.storageBuckets.delete('remove'),navigator.storageBuckets.delete('remove')]);
        let closed = false, stale = false;
        try { db.transaction('records'); } catch (error) { closed = error.name === 'InvalidStateError'; }
        stale = await result(factory.open('stale')).then(() => false, error => error.name === 'UnknownError');
        const fresh = await navigator.storageBuckets.open('remove');
        const databases = await fresh.indexedDB.databases();
        const value = await saved.blob.text();
        return {closed,stale,outcome:await outcome,databases,value};
    "#).await;
    assert_eq!(
        output,
        r#"{"closed":true,"stale":true,"outcome":"abort","databases":[],"value":"kept after deletion"}"#
    );
    assert_eq!(
        service(&manager)
            .owner_usage_bytes(identity.indexed_db_storage_key().as_bytes())
            .unwrap(),
        0
    );
    let replacement = page
        .storage_bucket_store
        .lock()
        .bucket_identity(&storage_key(), "remove")
        .unwrap();
    assert_ne!(replacement.bucket_id(), identity.bucket_id());
}

#[tokio::test]
async fn idb_sqlite_bucket_delete_completes_on_the_accepting_worker() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const url = URL.createObjectURL(new Blob([`
            (async () => {
                const bucket = await navigator.storageBuckets.open('worker-remove');
                const factory = bucket.indexedDB;
                const db = await new Promise((resolve,reject) => {
                    const request = factory.open('records',1);
                    request.onupgradeneeded = () => request.result.createObjectStore('records');
                    request.onerror = () => reject(request.error);
                    request.onsuccess = () => resolve(request.result);
                });
                const tx = db.transaction('records','readwrite');
                const done = new Promise((resolve,reject) => { tx.oncomplete = resolve; tx.onabort = () => reject(tx.error); });
                tx.objectStore('records').put('kept',1);
                await done;
                await navigator.storageBuckets.delete('worker-remove');
                let closed = false;
                try { db.transaction('records'); } catch (error) { closed = error.name === 'InvalidStateError'; }
                const stale = await new Promise(resolve => {
                    const request = factory.open('stale');
                    request.onerror = () => resolve(request.error.name);
                    request.onsuccess = () => resolve('unexpected-success');
                });
                const fresh = await navigator.storageBuckets.open('worker-remove');
                postMessage({closed,stale,databases:await fresh.indexedDB.databases()});
            })().catch(error => postMessage({error:String(error)}));
        `],{type:'text/javascript'}));
        const worker = new Worker(url);
        const response = await new Promise((resolve,reject) => {
            worker.onerror = event => reject(Error(event.message));
            worker.onmessage = event => resolve(event.data);
        });
        worker.terminate();
        URL.revokeObjectURL(url);
        return response;
    "#).await;
    assert_eq!(
        output,
        r#"{"closed":true,"stale":"UnknownError","databases":[]}"#
    );
}

#[tokio::test]
async fn idb_sqlite_bucket_delete_continues_after_the_accepting_page_retires() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    assert_eq!(run(&mut page, &loader, r#"
        const bucket = await navigator.storageBuckets.open('retire');
        const db = await new Promise((resolve,reject) => {
            const request = bucket.indexedDB.open('records',1);
            request.onupgradeneeded = () => request.result.createObjectStore('records').put('kept',1);
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
        });
        globalThis.retainedBucketDb = db;
        return 'seeded';
    "#).await, r#""seeded""#);
    let store = page.storage_bucket_store.clone();
    let identity = store
        .lock()
        .bucket_identity(&storage_key(), "retire")
        .unwrap();
    let storage = store.lock().storage_service();
    // Keep maintenance before the deletion itself starts, so retirement is
    // tested before metadata publication as well as before SQL cleanup.
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let default = moli_storage_service::StorageBucketLocator::default_bucket(storage_key());
    // Contention selects the maintenance queue; the uncontended fast path
    // uses the distinct file-I/O queue and would not gate this operation.
    let select_maintenance = storage.reserve_quota_commit(&default);
    storage
        .dispatch_opfs_quota_commit(
            default,
            move |_| {
                entered.send(()).unwrap();
                resume
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
            },
            |result| {
                result.unwrap();
            },
        )
        .unwrap();
    drop(select_maintenance);
    waiting
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    // The final marker must select the same maintenance queue as deletion.
    let quota = storage.reserve_quota_commit(&identity.locator());
    page.eval(r#"
        globalThis.deletionSettled = false;
        navigator.storageBuckets.delete('retire').then(() => deletionSettled = true, () => deletionSettled = true);
        globalThis.v8StillUsable = 6 * 7;
    "#).unwrap();
    assert_eq!(
        page.eval("String(deletionSettled) + ':' + v8StillUsable")
            .unwrap(),
        "false:42"
    );
    assert!(store.lock().pending_deletions().is_empty());
    assert!(store.lock().bucket_identity_is_live(&identity));
    // This marker is behind deletion on the existing maintenance sequence.
    // Its receipt proves metadata publication and cleanup ran after Page drop.
    let (sender, receiver) = std::sync::mpsc::channel();
    storage
        .dispatch_opfs_quota_commit(
            identity.locator(),
            |_| (),
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    drop(page);
    release.send(()).unwrap();
    drop(quota);
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert!(store.lock().pending_deletions().is_empty());
    assert_eq!(
        service(&manager)
            .owner_usage_bytes(identity.indexed_db_storage_key().as_bytes())
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn idb_sqlite_bucket_delete_persists_new_identity_and_preserves_other_buckets() {
    let root = indexed_db_test_root("sqlite-bucket-delete");
    std::fs::create_dir_all(&root).unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut deleted = None;
    let mut replacement = None;
    for reopened in [false, true] {
        let manager = manager(Some(root.join("idb-lite")));
        let storage = moli_storage_service::StorageService::on_disk(root.join("opfs")).unwrap();
        let store =
            moli_storage_service::new_shared_json_storage_bucket_store_with_storage_service(
                root.join("buckets.json"),
                root.join("cache"),
                &manager,
                storage,
            )
            .unwrap();
        let mut page = page(&manager, &loader);
        page.set_storage_bucket_store(store.clone());
        if !reopened {
            deleted = Some(store.lock().open_bucket(&storage_key(), "remove").unwrap());
            assert_eq!(run(&mut page, &loader, r#"
                const create = async (factory,value) => {
                    const db = await new Promise((resolve,reject) => {
                        const request = factory.open('records',1);
                        request.onupgradeneeded = () => request.result.createObjectStore('records').put(value,1);
                        request.onerror = () => reject(request.error);
                        request.onsuccess = () => resolve(request.result);
                    });
                    db.close();
                };
                await create(indexedDB,'default');
                const foreign = await navigator.storageBuckets.open('foreign');
                await create(foreign.indexedDB,'foreign');
                const old = await navigator.storageBuckets.open('remove');
                await create(old.indexedDB,'discarded');
                await navigator.storageBuckets.delete('remove');
                const fresh = await navigator.storageBuckets.open('remove');
                if ((await fresh.indexedDB.databases()).length) throw Error('deleted records survived');
                await create(fresh.indexedDB,'replacement');
                return 'created';
            "#).await, r#""created""#);
            replacement = store.lock().bucket_identity(&storage_key(), "remove");
            assert_ne!(
                deleted.as_ref().unwrap().bucket_id(),
                replacement.as_ref().unwrap().bucket_id()
            );
        } else {
            assert_eq!(
                store.lock().bucket_identity(&storage_key(), "remove"),
                replacement
            );
            assert_eq!(run(&mut page, &loader, r#"
                const values = [];
                const foreign = await navigator.storageBuckets.open('foreign');
                const fresh = await navigator.storageBuckets.open('remove');
                for (const factory of [indexedDB,foreign.indexedDB,fresh.indexedDB]) {
                    const db = await new Promise((resolve,reject) => {
                        const request = factory.open('records');
                        request.onupgradeneeded = () => reject(Error('database was lost'));
                        request.onerror = () => reject(request.error);
                        request.onsuccess = () => resolve(request.result);
                    });
                    values.push(await result(db.transaction('records').objectStore('records').get(1)));
                    db.close();
                }
                return values;
            "#).await, r#"["default","foreign","replacement"]"#);
        }
        assert!(store.lock().pending_deletions().is_empty());
        assert_eq!(
            service(&manager)
                .owner_usage_bytes(
                    deleted
                        .as_ref()
                        .unwrap()
                        .indexed_db_storage_key()
                        .as_bytes()
                )
                .unwrap(),
            0
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
