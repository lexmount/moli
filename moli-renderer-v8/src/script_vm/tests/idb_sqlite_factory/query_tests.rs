use super::*;

#[tokio::test]
async fn idb_sqlite_get_all_batches_preserve_web_index_records_keys_and_request_order() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(
        &mut page,
        &loader,
        r#"
        const db = await open('batch-query', 1, db => {
            const store = db.createObjectStore('records', {keyPath:'id'});
            store.createIndex('group','group');
        });
        const write = db.transaction('records','readwrite');
        for (let id=1;id<=1005;id++) write.objectStore('records').put({id,group:id%2});
        await complete(write);
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        const index = store.index('group');
        const events = [];
        const all = store.getAll();
        const keys = index.getAllKeys();
        const records = index.getAllRecords({direction:'prev',count:1002});
        const deletion = store.delete(1005);
        const after = store.count();
        const requests = [all,keys,records,deletion,after].map((request, i) => {
            request.addEventListener('success', () => events.push(i));
            return result(request);
        });
        const [values,primaryKeys,entries,,count] = await Promise.all(requests);
        await complete(tx);
        const uniqueTx = db.transaction('records');
        const unique = await result(uniqueTx.objectStore('records').index('group')
            .getAllRecords({direction:'prevunique'}));
        await complete(uniqueTx);
        db.close();
        return {
            count,events,
            all:[values.length,values[0].id,values[1004].id],
            keys:[primaryKeys.length,primaryKeys[0],primaryKeys[501],primaryKeys[502],primaryKeys[1004]],
            records:[entries.length,entries[0].key,entries[0].primaryKey,entries[0].value.id,entries[1001].key,entries[1001].primaryKey],
            unique:unique.map(entry => [entry.key,entry.primaryKey,entry.value.id])
        };
        "#,
    )
    .await;
    assert_eq!(
        output,
        r#"{"count":1004,"events":[0,1,2,3,4],"all":[1005,1,1005],"keys":[1005,2,1004,1,1005],"records":[1002,1,1005,1005,0,8],"unique":[[1,1,1],[0,2,2]]}"#
    );
    drop(page);
    manager.lock().shutdown_sqlite().unwrap();
}
