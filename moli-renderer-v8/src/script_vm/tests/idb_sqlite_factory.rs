use super::*;
use moli_indexeddb::sqlite::service::{ServiceClient, ServiceConfig};

mod admission_tests;
mod deletion_tests;
mod maintenance_tests;
mod quota_tests;
mod schema_tests;
mod version_tests;

const ORIGIN: &str = "https://idb-sql-factory.test";

fn manager(root: Option<std::path::PathBuf>) -> crate::SharedIndexedDbManager {
    let mut config = ServiceConfig::default();
    config.executor.workers = 1;
    crate::new_sqlite_indexed_db_manager(root, config).unwrap()
}

fn page(
    manager: &crate::SharedIndexedDbManager,
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut page = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse(ORIGIN).unwrap(),
        loader,
    );
    page.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(manager)));
    page.set_storage_bucket_store(
        crate::new_shared_storage_bucket_store_with_indexed_db_manager(manager),
    );
    page
}

fn storage_key() -> String {
    moli_storage_key::MoliStorageKey::first_party_from_url(&url::Url::parse(ORIGIN).unwrap(), None)
        .serialized_storage_key()
}

fn service(manager: &crate::SharedIndexedDbManager) -> ServiceClient {
    manager.lock().sqlite_service()
}

async fn run(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    body: &str,
) -> String {
    page.eval(&format!(r#"
        globalThis.sqlFactoryResult = {{done:false}};
        (async () => {{
            const open = (name, version, upgrade) => new Promise((resolve,reject) => {{
                const request = indexedDB.open(name, version);
                request.onerror = () => reject(request.error);
                request.onsuccess = () => resolve(request.result);
                request.onupgradeneeded = event => upgrade?.(request.result, request.transaction, event);
            }});
            const complete = transaction => new Promise((resolve,reject) => {{
                transaction.oncomplete = resolve;
                transaction.onabort = () => reject(transaction.error || Error('transaction aborted'));
            }});
            const result = request => new Promise((resolve,reject) => {{
                request.onsuccess = () => resolve(request.result);
                request.onerror = () => reject(request.error);
            }});
            {body}
        }})().then(value => sqlFactoryResult = {{done:true,value}}, error => sqlFactoryResult = {{done:true,error:String(error),stack:error?.stack}});
    "#)).unwrap();
    advance_page_task_executor_until_eval_equals(
        page,
        loader,
        "String(sqlFactoryResult.done)",
        "true",
        "SQL factory",
    )
    .await;
    assert_eq!(
        page.eval("sqlFactoryResult.error || ''").unwrap(),
        "",
        "{}",
        page.eval("JSON.stringify(sqlFactoryResult)").unwrap()
    );
    page.eval("JSON.stringify(sqlFactoryResult.value)").unwrap()
}

#[tokio::test]
async fn idb_sqlite_factory_opens_upgrades_and_executes_real_web_requests() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(
        &mut page,
        &loader,
        r#"
        const events = [];
        const db = await open('records', 1, (db,tx,event) => {
            events.push(`upgrade:${event.oldVersion}:${event.newVersion}:${db.version}`);
            const store = db.createObjectStore('records', {keyPath:'id',autoIncrement:true});
            store.createIndex('group','group');
            store.put({group:'alpha'});
            tx.oncomplete = () => events.push('upgrade-complete');
            Promise.resolve().then(() => events.push('upgrade-microtask'));
        });
        events.push('open-success');
        const tx = db.transaction('records','readwrite');
        const value = {group:'beta'};
        tx.objectStore('records').put(value);
        value.group = 'mutated';
        await complete(tx);
        const read = db.transaction('records');
        const values = await result(read.objectStore('records').index('group').getAll());
        const infos = await indexedDB.databases();
        db.close();
        return {events,values,infos};
    "#,
    )
    .await;
    assert_eq!(
        output,
        r#"{"events":["upgrade:0:1:1","upgrade-microtask","upgrade-complete","open-success"],"values":[{"group":"alpha","id":1},{"group":"beta","id":2}],"infos":[{"name":"records","version":1}]}"#
    );
    let service = service(&manager);
    assert_eq!(
        service
            .database_version(ORIGIN.as_bytes(), "records")
            .unwrap(),
        None,
        "origin projections must not alias a full storage key"
    );
    assert_eq!(
        service
            .database_version(storage_key().as_bytes(), "records")
            .unwrap(),
        Some(1)
    );
    assert!(service.owner_usage_bytes(storage_key().as_bytes()).unwrap() > 0);
    assert_eq!(service.owner_usage_bytes(ORIGIN.as_bytes()).unwrap(), 0);
}

#[tokio::test]
async fn idb_sqlite_factory_serializes_opens_and_waits_for_versionchange_microtasks() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const events = [];
        const first = await open('fifo',1, db => db.createObjectStore('records'));
        first.onversionchange = event => {
            events.push(`vc:${event.oldVersion}:${event.newVersion}`);
            Promise.resolve().then(() => { events.push('vc-microtask'); first.close(); });
        };
        const upgraded = new Promise((resolve,reject) => {
            const request = indexedDB.open('fifo',2);
            request.onblocked = () => { events.push('unexpected-blocked'); reject(Error('close in versionchange should not block')); };
            request.onerror = () => reject(request.error);
            request.onupgradeneeded = event => {
                events.push(`upgrade:${event.oldVersion}:${event.newVersion}`);
                request.transaction.oncomplete = () => events.push('upgrade-complete');
            };
            request.onsuccess = () => { events.push('upgrade-success'); request.result.close(); resolve(); };
        });
        const queued = new Promise((resolve,reject) => {
            const request = indexedDB.open('fifo');
            request.onerror = () => reject(request.error);
            request.onupgradeneeded = () => reject(Error('queued versionless open must observe version 2'));
            request.onsuccess = () => { events.push(`same-success:${request.result.version}`); request.result.close(); resolve(); };
        });
        await Promise.all([upgraded,queued]);
        return events;
    "#).await;
    assert_eq!(
        output,
        r#"["vc:1:2","vc-microtask","upgrade:1:2","upgrade-complete","upgrade-success","same-success:2"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_factory_delete_blocks_once_and_recreates_a_fresh_database() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const events = [];
        const db = await open('remove',3, db => db.createObjectStore('old'));
        db.onversionchange = event => {
            events.push(`vc:${event.oldVersion}:${event.newVersion}`);
            Promise.resolve().then(() => events.push('vc-microtask'));
        };
        await new Promise((resolve,reject) => {
            const request = indexedDB.deleteDatabase('remove');
            request.onerror = () => reject(request.error);
            request.onblocked = event => { events.push(`blocked:${event.oldVersion}:${event.newVersion}`); db.close(); };
            request.onsuccess = event => {
                events.push(`deleted:${event instanceof IDBVersionChangeEvent}:${event.oldVersion}:${event.newVersion}`);
                resolve();
            };
        });
        if ((await indexedDB.databases()).length) throw Error('deleted database still listed');
        const fresh = await open('remove',undefined,(db,tx,event) => events.push(`fresh:${event.oldVersion}:${event.newVersion}`));
        if (fresh.objectStoreNames.length) throw Error('old schema survived deletion');
        events.push(`opened:${fresh.version}`);
        fresh.close();
        return events;
    "#).await;
    assert_eq!(
        output,
        r#"["vc:3:null","vc-microtask","blocked:3:null","deleted:true:3:null","fresh:0:1","opened:1"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_factory_aborted_upgrade_rolls_back_before_following_open_and_version_error() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const initial = await open('abort',3, db => db.createObjectStore('original').put('saved',1));
        initial.onversionchange = () => initial.close();
        const aborted = open('abort',4,(db,tx) => {
            db.deleteObjectStore('original');
            db.createObjectStore('discarded').put('discarded',1);
            tx.abort();
        }).then(() => { throw Error('aborted upgrade succeeded'); }, error => error.name);
        const reopened = open('abort',3);
        const error = await aborted;
        const db = await reopened;
        const schema = Array.from(db.objectStoreNames);
        const value = await result(db.transaction('original').objectStore('original').get(1));
        const lower = await open('abort',2).then(() => 'unexpected-success', error => error.name);
        db.close();
        return {error,version:db.version,schema,value,lower};
    "#).await;
    assert_eq!(
        output,
        r#"{"error":"AbortError","version":3,"schema":["original"],"value":"saved","lower":"VersionError"}"#
    );
}

#[tokio::test]
async fn idb_sqlite_factory_thrown_upgrade_rolls_back_before_the_next_open() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const events = [];
        const failed = open('throw',1,(db,tx) => {
            db.createObjectStore('discarded').put('discarded',1);
            tx.onabort = () => events.push('abort');
            tx.oncomplete = () => { throw Error('throwing upgrade committed'); };
            throw Error('deliberate upgrade failure');
        }).then(() => { throw Error('throwing upgrade succeeded'); }, error => events.push(error.name));
        const next = open('throw',undefined,(db,tx,event) => {
            events.push(`next:${event.oldVersion}:${event.newVersion}:${db.objectStoreNames.length}`);
            db.createObjectStore('kept').put('saved',1);
        });
        await failed;
        const db = await next;
        const value = await result(db.transaction('kept').objectStore('kept').get(1));
        const schema = Array.from(db.objectStoreNames);
        db.close();
        return {events,value,schema};
    "#).await;
    assert_eq!(
        output,
        r#"{"events":["abort","AbortError","next:0:1:0"],"value":"saved","schema":["kept"]}"#
    );
}

fn terminate_upgrade(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.terminate_execution();
}

#[tokio::test]
async fn idb_sqlite_factory_explicit_upgrade_commit_survives_a_throwing_listener() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(
        &mut page,
        &loader,
        r#"
        const db = await open('commit-throw',1,(db,tx) => {
            db.createObjectStore('kept').put('saved',1);
            tx.commit();
            throw Error('after explicit commit');
        });
        const value = await result(db.transaction('kept').objectStore('kept').get(1));
        db.close();
        return {version:db.version,value};
    "#,
    )
    .await;
    assert_eq!(output, r#"{"version":1,"value":"saved"}"#);
}

#[tokio::test]
async fn idb_sqlite_factory_terminated_upgrade_respects_commit_and_releases_the_next_open() {
    for explicit in [false, true] {
        let manager = manager(None);
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut page = page(&manager, &loader);
        run(
            &mut page,
            &loader,
            r#"
        const db = await open('terminate',3, db => db.createObjectStore('original').put('saved',1));
        db.close();
        return 'seeded';
    "#,
        )
        .await;
        page.with_default_context_scope(|scope, _| {
            let callback = v8::Function::new(scope, terminate_upgrade).unwrap();
            scope
                .get_current_context()
                .global(scope)
                .set(
                    scope,
                    crate::util::v8str(scope, "terminateUpgrade").into(),
                    callback.into(),
                )
                .unwrap();
            Ok(())
        })
        .unwrap();
        page.eval(&r#"
        globalThis.upgradeEvents = [];
        globalThis.upgradeRecoveryDone = false;
        const interrupted = indexedDB.open('terminate',4);
        interrupted.onupgradeneeded = () => {
            upgradeEvents.push('upgrade');
            interrupted.transaction.onabort = () => upgradeEvents.push('abort');
            interrupted.transaction.oncomplete = () => upgradeEvents.push('complete');
            interrupted.result.deleteObjectStore('original');
            interrupted.result.createObjectStore('discarded');
            if (EXPLICIT_COMMIT) interrupted.transaction.commit();
            terminateUpgrade();
            upgradeEvents.push('termination-returned');
            while (true) {}
        };
        interrupted.onerror = () => upgradeEvents.push('failed:' + interrupted.error.name);
        interrupted.onsuccess = () => { upgradeEvents.push('opened:' + interrupted.result.version); interrupted.result.close(); };
        const next = indexedDB.open('terminate');
        next.onerror = () => { upgradeEvents.push('next-error:' + next.error.name); upgradeRecoveryDone = true; };
        next.onupgradeneeded = () => upgradeEvents.push('unexpected-upgrade');
        next.onsuccess = () => {
            const db = next.result;
            upgradeEvents.push('next:' + db.version + ':' + Array.from(db.objectStoreNames).join(','));
            const name = db.objectStoreNames[0];
            const read = db.transaction(name).objectStore(name).get(1);
            read.onsuccess = () => { upgradeEvents.push('value:' + read.result); db.close(); upgradeRecoveryDone = true; };
        };
    "#.replace("EXPLICIT_COMMIT", if explicit { "true" } else { "false" })).unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
        while page.eval("String(upgradeRecoveryDone)").unwrap() != "true" {
            match wait_for_one_selected_page_task_executor_test_turn_until(
                &mut page, &loader, deadline,
            )
            .await
            {
                Ok(()) => {}
                Err(error) => {
                    let message = format!("{error:#}").to_lowercase();
                    assert!(
                        message.contains("terminat") || message.contains("interrupted"),
                        "{message}; events={}",
                        page.eval("JSON.stringify(upgradeEvents)").unwrap()
                    );
                }
            }
        }
        assert_eq!(
            page.eval("JSON.stringify(upgradeEvents)").unwrap(),
            if explicit {
                r#"["upgrade","complete","opened:4","next:4:discarded","value:undefined"]"#
            } else {
                r#"["upgrade","abort","failed:AbortError","next:3:original","value:saved"]"#
            }
        );
    }
}

#[tokio::test]
async fn idb_sqlite_factory_restarts_disk_storage_without_reading_the_legacy_directory() {
    let root = indexed_db_test_root("sqlite-factory");
    let legacy = root.join("indexeddb");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("broken.json"),
        b"deliberately invalid legacy JSON",
    )
    .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let generation;
    {
        let manager = manager(Some(root.join("idb-lite")));
        let mut page = page(&manager, &loader);
        assert_eq!(run(&mut page,&loader,r#"
            const db = await open('disk',5, db => db.createObjectStore('records').put('persisted',1));
            db.close();
            return db.version;
        "#).await,"5");
        let service = service(&manager);
        let (sender, receiver) = std::sync::mpsc::channel();
        let client = service
            .resolve(storage_key().into_bytes(), "disk".into(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(8))
            .unwrap()
            .result
            .unwrap();
        generation = client.identity().generation;
    }
    {
        let manager = manager(Some(root.join("idb-lite")));
        let mut page = page(&manager, &loader);
        assert_eq!(run(&mut page,&loader,r#"
            const db = await open('disk',undefined,()=>{throw Error('restarted database unexpectedly upgraded')});
            const value = await result(db.transaction('records').objectStore('records').get(1));
            const infos = await indexedDB.databases();
            db.close();
            return {version:db.version,value,infos};
        "#).await,r#"{"version":5,"value":"persisted","infos":[{"name":"disk","version":5}]}"#);
        let service = service(&manager);
        let (sender, receiver) = std::sync::mpsc::channel();
        let client = service
            .resolve(storage_key().into_bytes(), "disk".into(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(8))
            .unwrap()
            .result
            .unwrap();
        assert_eq!(client.identity().generation, generation);
    }
    assert_eq!(
        std::fs::read(legacy.join("broken.json")).unwrap(),
        b"deliberately invalid legacy JSON"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn idb_sqlite_factory_coordinates_a_worker_connection_and_page_upgrade() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page,&loader,r#"
        const events = [];
        const url = URL.createObjectURL(new Blob([`
            const request = indexedDB.open('worker-shared',1);
            request.onupgradeneeded = () => request.result.createObjectStore('records');
            request.onerror = () => postMessage('error:' + request.error?.name);
            request.onsuccess = () => {
                const db = request.result;
                db.onversionchange = event => {
                    postMessage('vc:' + event.oldVersion + ':' + event.newVersion);
                    Promise.resolve().then(() => { db.close(); postMessage('closed'); });
                };
                postMessage('ready');
            };
        `],{type:'text/javascript'}));
        const worker = new Worker(url);
        await new Promise((resolve,reject) => {
            worker.onerror = event => reject(Error(event.message));
            worker.onmessage = event => {
                events.push(event.data);
                if(event.data === 'ready') resolve();
                if(String(event.data).startsWith('error:')) reject(Error(event.data));
            };
        });
        const db = await new Promise((resolve,reject) => {
            const request = indexedDB.open('worker-shared',2);
            request.onblocked = () => reject(Error('worker close in microtask should not block'));
            request.onerror = () => reject(request.error);
            request.onupgradeneeded = event => events.push('upgrade:' + event.oldVersion + ':' + event.newVersion);
            request.onsuccess = () => resolve(request.result);
        });
        const version = db.version;
        db.close();
        worker.terminate();
        URL.revokeObjectURL(url);
        return {version,ready:events.includes('ready'),upgraded:events.includes('upgrade:1:2')};
    "#).await;
    assert_eq!(output, r#"{"version":2,"ready":true,"upgraded":true}"#);
}

#[tokio::test]
async fn idb_sqlite_factory_worker_retirement_rolls_back_upgrade_before_page_open() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page,&loader,r#"
        const url = URL.createObjectURL(new Blob([`
            const request = indexedDB.open('retired-upgrade',2);
            request.onupgradeneeded = () => {
                const store = request.result.createObjectStore('discarded');
                // Keep this real upgrade alive until its owner is retired.
                const keepActive = () => { store.get(0).onsuccess = keepActive; };
                keepActive();
                postMessage('upgrade');
            };
            request.onerror = () => postMessage('error:' + request.error?.name);
            request.onsuccess = () => postMessage('unexpected-commit');
        `],{type:'text/javascript'}));
        const worker = new Worker(url);
        await new Promise((resolve,reject) => {
            worker.onerror = event => reject(Error(event.message));
            worker.onmessage = event => event.data === 'upgrade' ? resolve() : reject(Error(event.data));
        });
        worker.terminate();
        URL.revokeObjectURL(url);
        let oldVersion;
        const db = await open('retired-upgrade',undefined,(db,tx,event) => {
            oldVersion = event.oldVersion;
            if (db.objectStoreNames.length) throw Error('retired worker published schema');
            db.createObjectStore('kept').put('saved',1);
        });
        const value = await result(db.transaction('kept').objectStore('kept').get(1));
        const schema = Array.from(db.objectStoreNames);
        db.close();
        return {oldVersion,version:db.version,value,schema};
    "#).await;
    assert_eq!(
        output,
        r#"{"oldVersion":0,"version":1,"value":"saved","schema":["kept"]}"#
    );
}

#[tokio::test]
async fn idb_sqlite_popup_close_retires_its_connection_and_preserves_the_opener() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const keeper = await open('opener-retained', 1, db => db.createObjectStore('records').put('opener',1));
        const popup = window.open('about:blank');
        const opened = new Promise((resolve,reject) => {
            const listener = event => {
                if (event.source !== popup) return;
                window.removeEventListener('message',listener);
                if (event.data !== 'opened') reject(Error(event.data)); else resolve();
            };
            window.addEventListener('message',listener);
        });
        popup.onmessage = async () => {
            try {
                // Keep the wrapper reachable in the shared realm: closing the
                // popup must retire its native connection even without GC.
                globalThis.popupHeldDatabase = await open('popup-retired',1, db => db.createObjectStore('records').put('popup',1));
                popup.opener.postMessage('opened','*');
            } catch (error) { popup.opener.postMessage(String(error),'*'); }
        };
        popup.postMessage('open','*');
        await opened;
        popup.close();
        const upgraded = await new Promise((resolve,reject) => {
            const request = indexedDB.open('popup-retired',2);
            request.onblocked = () => reject(Error('closed popup still blocks upgrade'));
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
        });
        const value = await result(upgraded.transaction('records').objectStore('records').get(1));
        const retained = await result(keeper.transaction('records').objectStore('records').get(1));
        upgraded.close();
        keeper.close();
        return {closed:popup.closed,version:upgraded.version,value,retained};
    "#).await;
    assert_eq!(
        output,
        r#"{"closed":true,"version":2,"value":"popup","retained":"opener"}"#
    );
}
