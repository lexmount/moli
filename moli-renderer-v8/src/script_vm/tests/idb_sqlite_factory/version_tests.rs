use super::*;

#[tokio::test]
async fn idb_sqlite_factory_preserves_large_versions_in_events_catalog_and_requests() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page,&loader,r#"
        const versions = [2 ** 31, 2 ** 32, Number.MAX_SAFE_INTEGER];
        let previous = 0;
        for (const version of versions) {
            const db = await open('large',version,(db,tx,event) => {
                if (event.oldVersion !== previous || event.newVersion !== version || db.version !== version)
                    throw Error('upgrade version lost precision');
                if (!previous) db.createObjectStore('records');
                tx.objectStore('records').put('value:' + version,1);
            });
            if (db.version !== version) throw Error('success version changed');
            const value = await result(db.transaction('records').objectStore('records').get(1));
            if (value !== 'value:' + version) throw Error('record changed');
            db.close();
            const same = await open('large',undefined,()=>{ throw Error('versionless open upgraded'); });
            if (same.version !== version) throw Error('versionless open lost precision');
            same.close();
            const infos = await indexedDB.databases();
            if (infos.length !== 1 || infos[0].version !== version) throw Error('catalog lost precision');
            previous = version;
        }
        const lower = await open('large',Number.MAX_SAFE_INTEGER - 1).then(()=>'unexpected-success',error=>error.name);
        const invalid = [0,-1,NaN,Infinity,2 ** 53,2 ** 63,2 ** 64 - 2048,2 ** 64,1n].map(version => {
            try { indexedDB.open('invalid',version); return 'unexpected-success'; }
            catch(error) { return error.name; }
        });
        return {roundTrips:versions.length,lower,invalid};
    "#).await;
    assert_eq!(
        output,
        r#"{"roundTrips":3,"lower":"VersionError","invalid":["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );
    assert_eq!(
        service(&manager)
            .database_version(storage_key().as_bytes(), "large")
            .unwrap(),
        Some((1_u64 << 53) - 1)
    );
}

#[tokio::test]
async fn idb_sqlite_factory_reopens_the_largest_webidl_version_from_disk() {
    let root = indexed_db_test_root("sqlite-u64-version");
    std::fs::create_dir_all(&root).unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for reopened in [false, true] {
        let manager = manager(Some(root.join("idb-lite")));
        let mut page = page(&manager, &loader);
        let body = r#"
            const expected = Number.MAX_SAFE_INTEGER;
            const db = await open('disk-large',OPEN_VERSION,(db) => {
                if (IS_REOPEN) throw Error('restarted database unexpectedly upgraded');
                db.createObjectStore('records').put('persisted',1);
            });
            const value = await result(db.transaction('records').objectStore('records').get(1));
            const infos = await indexedDB.databases();
            db.close();
            return {version:db.version === expected,value,catalog:infos[0].version === expected};
        "#
        .replace(
            "OPEN_VERSION",
            if reopened {
                "undefined"
            } else {
                "Number.MAX_SAFE_INTEGER"
            },
        )
        .replace("IS_REOPEN", if reopened { "true" } else { "false" });
        assert_eq!(
            run(&mut page, &loader, &body).await,
            r#"{"version":true,"value":"persisted","catalog":true}"#
        );
        assert_eq!(
            service(&manager)
                .database_version(storage_key().as_bytes(), "disk-large")
                .unwrap(),
            Some((1_u64 << 53) - 1)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
