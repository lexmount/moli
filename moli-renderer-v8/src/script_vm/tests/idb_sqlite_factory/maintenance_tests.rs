use super::*;

#[tokio::test]
async fn idb_sqlite_bucket_expiry_reopens_an_empty_identity_and_preserves_the_default_database() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const requestResult = request => new Promise((resolve,reject) => {
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
        });
        const bucketOpen = bucket => new Promise((resolve,reject) => {
            const request = bucket.indexedDB.open('records',1);
            request.onupgradeneeded = () => request.result.createObjectStore('records').put('kept',1);
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
        });
        const defaults = await open('default',1, db => db.createObjectStore('records').put('default-kept',1));
        const bucket = await navigator.storageBuckets.open('expire');
        const oldFactory = bucket.indexedDB;
        const oldDb = await bucketOpen(bucket);
        const delivered = await result(oldDb.transaction('records').objectStore('records').get(1));
        await bucket.setExpires(1);
        let stale;
        try { await bucket.estimate(); } catch (error) { stale = error.name; }
        const replacement = await navigator.storageBuckets.open('expire');
        const databases = await replacement.indexedDB.databases();
        let closed;
        try { oldDb.transaction('records'); } catch (error) { closed = error.name; }
        let oldFactoryError;
        try { await requestResult(oldFactory.open('records')); } catch (error) { oldFactoryError = error.name; }
        const kept = await result(defaults.transaction('records').objectStore('records').get(1));
        return {stale,closed,oldFactoryError,databases,delivered,kept,keys:await navigator.storageBuckets.keys()};
    "#).await;
    assert_eq!(
        output,
        r#"{"stale":"UnknownError","closed":"InvalidStateError","oldFactoryError":"UnknownError","databases":[],"delivered":"kept","kept":"default-kept","keys":["expire"]}"#
    );
}

#[tokio::test]
async fn idb_sqlite_worker_keys_finishes_expired_bucket_cleanup() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const url = URL.createObjectURL(new Blob([`
            (async () => {
                const bucket = await navigator.storageBuckets.open('expire-worker');
                const db = await new Promise((resolve,reject) => {
                    const request = bucket.indexedDB.open('records',1);
                    request.onupgradeneeded = () => request.result.createObjectStore('records').put('kept',1);
                    request.onerror = () => reject(request.error);
                    request.onsuccess = () => resolve(request.result);
                });
                await bucket.setExpires(1);
                const keys = await navigator.storageBuckets.keys();
                let closed;
                try { db.transaction('records'); } catch (error) { closed = error.name; }
                const replacement = await navigator.storageBuckets.open('expire-worker');
                return {keys,closed,databases:await replacement.indexedDB.databases()};
            })().then(value => postMessage(value), error => postMessage({error:String(error)}));
        `],{type:'text/javascript'}));
        const worker = new Worker(url);
        const response = await new Promise((resolve,reject) => {
            worker.onmessage = event => resolve(event.data);
            worker.onerror = event => reject(Error(event.message));
        });
        worker.terminate();
        URL.revokeObjectURL(url);
        return response;
    "#).await;
    assert_eq!(
        output,
        r#"{"keys":[],"closed":"InvalidStateError","databases":[]}"#
    );
}
