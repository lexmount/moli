use super::*;

fn assert_initial_worker_auth_network_headers(headers: Option<&[(String, String)]>) {
    let headers = headers.expect("worker auth transport request headers");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("host") && !value.is_empty()),
        "worker auth transport headers should contain Host: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("authorization")),
        "the browser-visible auth request observation must remain the initial unauthenticated exchange: {headers:?}"
    );
}

#[tokio::test]
async fn worker_request_constructor_resolves_relative_url_against_worker_script_url() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const request = new Request("./data.txt");
        postMessage(request.url);
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""http://127.0.0.1/worker/data.txt""#
    );
}

#[tokio::test]
async fn blob_worker_request_constructor_rejects_relative_url() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let constructed = false;
        try {
            new Request("./data.txt");
            constructed = true;
        } catch (error) {
            postMessage({ constructed, name: error.name });
            close();
        }
        "#
        .into(),
        "blob:http://127.0.0.1/worker-script".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"constructed":false,"name":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_url_and_search_params_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const url = new URL("./data.txt?x=1&x=2", "http://127.0.0.1/worker/main.js");
        const params = new URLSearchParams("a=1&a=2&b=3");
        postMessage({
            urlCtor: typeof URL,
            searchParamsCtor: typeof URLSearchParams,
            href: url.href,
            origin: url.origin,
            pathname: url.pathname,
            search: url.search,
            xValues: url.searchParams.getAll("x"),
            paramsString: params.toString(),
            canParseRelative: URL.canParse("./next.js", "http://127.0.0.1/worker/main.js"),
            canParseInvalidBase: URL.canParse("./next.js", "not a url"),
            urlTag: Object.prototype.toString.call(url),
            paramsTag: Object.prototype.toString.call(params),
        });
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"urlCtor":"function","searchParamsCtor":"function","href":"http://127.0.0.1/worker/data.txt?x=1&x=2","origin":"http://127.0.0.1","pathname":"/worker/data.txt","search":"?x=1&x=2","xValues":["1","2"],"paramsString":"a=1&a=2&b=3","canParseRelative":true,"canParseInvalidBase":false,"urlTag":"[object URL]","paramsTag":"[object URLSearchParams]"}"#
    );
}

#[tokio::test]
async fn worker_navigator_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        "use strict";
        let assignError = null;
        try {
            navigator.appName = "";
        } catch (error) {
            assignError = error.name;
        }
        const proto = Object.getPrototypeOf(navigator);
        const userAgent = Object.getOwnPropertyDescriptor(proto, "userAgent");
        const storage = navigator.storage;
        const serviceWorker = navigator.serviceWorker;
        const illegalReceiverProbe = StorageManager.prototype.estimate.call({}).then(
            () => "resolved",
            (error) => error && error.name
        );
        const spoofedReceiverProbe = StorageManager.prototype.estimate.call({
            __moliStorageManagerBrand: true
        }).then(
            () => "resolved",
            (error) => error && error.name
        );
        Promise.all([
            storage.persisted(),
            storage.estimate(),
            illegalReceiverProbe,
            spoofedReceiverProbe
        ]).then(([persisted, estimate, illegalReceiverError, spoofedReceiverError]) => {
        postMessage({
            navigatorOwn: Object.prototype.hasOwnProperty.call(self, "navigator"),
            ctorOwn: Object.prototype.hasOwnProperty.call(self, "WorkerNavigator"),
            navigatorType: typeof navigator,
            ctorType: typeof WorkerNavigator,
            ctorName: navigator.constructor && navigator.constructor.name,
            protoCtor: proto && proto.constructor && proto.constructor.name,
            tag: Object.prototype.toString.call(navigator),
            instanceofWorkerNavigator: navigator instanceof WorkerNavigator,
            ownUserAgent: Object.prototype.hasOwnProperty.call(navigator, "userAgent"),
            userAgentGetterType: typeof userAgent?.get,
            appCodeName: navigator.appCodeName,
            appName: navigator.appName,
            appVersionStartsWithWebKit: navigator.appVersion.indexOf("WebKit") === 0,
            platformType: typeof navigator.platform,
            userAgentStartsWithWebKit: navigator.userAgent.indexOf("WebKit") === 0,
            product: navigator.product,
            language: navigator.language,
            languages: Array.from(navigator.languages || []),
            onLineType: typeof navigator.onLine,
            hardwareConcurrencyPositive: navigator.hardwareConcurrency > 0,
            deviceMemoryPositive: typeof navigator.deviceMemory === "number" && navigator.deviceMemory > 0,
            assignError,
            serviceWorkerType: typeof serviceWorker,
            serviceWorkerControllerIsNull: serviceWorker.controller === null,
            serviceWorkerRegisterType: typeof serviceWorker.register,
            serviceWorkerGetRegistrationType: typeof serviceWorker.getRegistration,
            serviceWorkerAddEventListenerType: typeof serviceWorker.addEventListener,
            serviceWorkerOncontrollerchangeIsNull: serviceWorker.oncontrollerchange === null,
            storageCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageManager"),
            storageEstimateCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageEstimate"),
            storageType: typeof storage,
            storageTag: Object.prototype.toString.call(storage),
            storageInstanceof: storage instanceof StorageManager,
            storageOwnEstimate: Object.prototype.hasOwnProperty.call(storage, "estimate"),
            storageOwnPersisted: Object.prototype.hasOwnProperty.call(storage, "persisted"),
            storageOwnPersist: Object.prototype.hasOwnProperty.call(storage, "persist"),
            storageOwnGetDirectory: Object.prototype.hasOwnProperty.call(storage, "getDirectory"),
            storageKeys: Object.keys(storage),
            storageProtoPersisted: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "persisted"),
            storageProtoEstimate: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "estimate"),
            storageProtoPersist: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "persist"),
            storageProtoGetDirectory: Object.prototype.hasOwnProperty.call(StorageManager.prototype, "getDirectory"),
            storagePersistedName: storage.persisted && storage.persisted.name,
            storagePersistedLength: storage.persisted && storage.persisted.length,
            storageEstimateName: storage.estimate && storage.estimate.name,
            storageEstimateLength: storage.estimate && storage.estimate.length,
            storageGetDirectoryName: storage.getDirectory && storage.getDirectory.name,
            storageGetDirectoryLength: storage.getDirectory && storage.getDirectory.length,
            storagePersistType: typeof storage.persist,
            persisted,
            estimateTag: Object.prototype.toString.call(estimate),
            estimateKeys: Object.keys(estimate),
            estimateQuota: estimate.quota,
            estimateUsage: estimate.usage,
            estimateUsageDetailsTag: Object.prototype.toString.call(estimate.usageDetails),
            estimateUsageDetailsKeys: Object.keys(estimate.usageDetails),
            illegalReceiverError,
            spoofedReceiverError,
        });
        close();
        });
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let actual = expect_post_json(msg);
    assert!(actual.contains(r#""languages":["en-US","en"]"#));
    assert_eq!(
        actual.replace(r#""languages":["en-US","en"]"#, r#""languages":["en-US"]"#,),
        r#"{"navigatorOwn":true,"ctorOwn":true,"navigatorType":"object","ctorType":"function","ctorName":"WorkerNavigator","protoCtor":"WorkerNavigator","tag":"[object WorkerNavigator]","instanceofWorkerNavigator":true,"ownUserAgent":false,"userAgentGetterType":"function","appCodeName":"Mozilla","appName":"Netscape","appVersionStartsWithWebKit":false,"platformType":"string","userAgentStartsWithWebKit":false,"product":"Gecko","language":"en-US","languages":["en-US"],"onLineType":"boolean","hardwareConcurrencyPositive":true,"deviceMemoryPositive":true,"assignError":"TypeError","serviceWorkerType":"object","serviceWorkerControllerIsNull":true,"serviceWorkerRegisterType":"function","serviceWorkerGetRegistrationType":"function","serviceWorkerAddEventListenerType":"function","serviceWorkerOncontrollerchangeIsNull":true,"storageCtorOwn":true,"storageEstimateCtorOwn":true,"storageType":"object","storageTag":"[object StorageManager]","storageInstanceof":true,"storageOwnEstimate":false,"storageOwnPersisted":false,"storageOwnPersist":false,"storageOwnGetDirectory":false,"storageKeys":[],"storageProtoPersisted":true,"storageProtoEstimate":true,"storageProtoPersist":false,"storageProtoGetDirectory":true,"storagePersistedName":"persisted","storagePersistedLength":0,"storageEstimateName":"estimate","storageEstimateLength":0,"storageGetDirectoryName":"getDirectory","storageGetDirectoryLength":0,"storagePersistType":"undefined","persisted":false,"estimateTag":"[object StorageEstimate]","estimateKeys":["quota","usage","usageDetails"],"estimateQuota":1073741824,"estimateUsage":0,"estimateUsageDetailsTag":"[object Object]","estimateUsageDetailsKeys":[],"illegalReceiverError":"TypeError","spoofedReceiverError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_opfs_root_directory_and_file_handles_are_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const root = await navigator.storage.getDirectory();
            const directory = await root.getDirectoryHandle("worker-dir", { create: true });
            const file = await directory.getFileHandle("worker-file", { create: true });
            const writable = await file.createWritable();
            await writable.write("worker");
            await writable.close();
            const snapshot = await file.getFile();
            const entries = [];
            for await (const [name, child] of root) {
                entries.push(`${name}:${child.kind}`);
            }
            const resolved = await root.resolve(file);
            const concurrentIterator = directory.keys();
            const iteratorBatch = (await Promise.all([
                concurrentIterator.next(),
                concurrentIterator.next()
            ])).map(result => result.done ? "done" : result.value);
            await directory.removeEntry("worker-file");
            const removedFile = await directory.getFileHandle("worker-file").then(
                () => "present",
                error => error && error.name
            );
            await directory.remove();
            const removedDirectory = await root.getDirectoryHandle("worker-dir").then(
                () => "present",
                error => error && error.name
            );
            postMessage({
                root: [root.kind, root.name],
                brands: [
                    root instanceof FileSystemDirectoryHandle,
                    root instanceof FileSystemHandle,
                    file instanceof FileSystemFileHandle,
                    file instanceof FileSystemHandle,
                    snapshot instanceof File,
                    writable instanceof FileSystemWritableFileStream,
                    writable instanceof WritableStream
                ],
                file: [file.name, snapshot.name, snapshot.size],
                resolved,
                entries,
                iteratorBatch,
                removedFile,
                removedDirectory,
                syncAccessHandleConstructor: typeof FileSystemSyncAccessHandle
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"root":["directory",""],"brands":[true,true,true,true,true,true,true],"file":["worker-file","worker-file",6],"resolved":["worker-dir","worker-file"],"entries":["worker-dir:directory"],"iteratorBatch":["worker-file","done"],"removedFile":"NotFoundError","removedDirectory":"NotFoundError","syncAccessHandleConstructor":"function"}"#
    );
}

#[tokio::test]
async fn worker_opfs_handle_permissions_match_sandboxed_fixed_grants() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const outcome = async promise => {
                try {
                    return `resolved:${await promise}`;
                } catch (error) {
                    return `rejected:${error && error.name}`;
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("permission", { create: true });
            postMessage({
                shape: [
                    typeof FileSystemHandle.prototype.queryPermission,
                    FileSystemHandle.prototype.queryPermission.length,
                    typeof FileSystemHandle.prototype.requestPermission,
                    FileSystemHandle.prototype.requestPermission.length
                ],
                rootRead: await root.queryPermission(),
                fileReadwrite: await file.queryPermission({ mode: "readwrite" }),
                requested: await file.requestPermission({ mode: "readwrite" }),
                invalid: await outcome(file.queryPermission({ mode: "invalid" })),
                illegal: await outcome(
                    FileSystemHandle.prototype.requestPermission.call({})
                )
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-permission-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"shape":["function",0,"function",0],"rootRead":"granted","fileReadwrite":"granted","requested":"granted","invalid":"rejected:TypeError","illegal":"rejected:TypeError"}"#
    );
}

#[tokio::test]
async fn worker_opfs_concurrent_file_moves_follow_storage_owner_order() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const outcome = async promise => {
                try {
                    await promise;
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const source = await root.getDirectoryHandle("source", { create: true });
            const destination = await root.getDirectoryHandle("destination", { create: true });
            const file = await source.getFileHandle("before.txt", { create: true });
            const writer = await file.createWritable();
            await writer.write("worker ordered move");
            await writer.close();

            const firstMove = file.move(destination, "middle.txt");
            const nameWhilePending = file.name;
            const secondMove = file.move("after.txt");
            const snapshot = file.getFile();
            const resolved = root.resolve(file);
            const sameSelf = file.isSameEntry(file);
            const values = await Promise.all([
                firstMove,
                outcome(secondMove),
                snapshot,
                resolved,
                sameSelf
            ]);
            const retryAfterSettlement = await outcome(file.move("after.txt"));
            const sourceKeys = [];
            for await (const key of source.keys()) sourceKeys.push(key);
            const destinationKeys = [];
            for await (const key of destination.keys()) destinationKeys.push(key);
            postMessage({
                nameWhilePending,
                finalName: file.name,
                secondMove: values[1],
                retryAfterSettlement,
                text: await values[2].text(),
                resolved: values[3],
                sameSelf: values[4],
                sourceKeys,
                destinationKeys
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-move-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"nameWhilePending":"before.txt","finalName":"after.txt","secondMove":"NoModificationAllowedError","retryAfterSettlement":"resolved","text":"worker ordered move","resolved":["destination","middle.txt"],"sameSelf":true,"sourceKeys":[],"destinationKeys":["after.txt"]}"#
    );
}

#[tokio::test]
async fn worker_opfs_directory_move_uses_owner_order_and_subtree_locks() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const outcome = async promise => {
                try {
                    await promise;
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const source = await root.getDirectoryHandle("source", { create: true });
            const destination = await root.getDirectoryHandle("destination", { create: true });
            const nested = await source.getDirectoryHandle("nested", { create: true });
            const file = await nested.getFileHandle("file.txt", { create: true });
            const writer = await file.createWritable();
            await writer.write("worker directory move");
            await writer.close();

            const lock = await file.createWritable({ keepExistingData: true });
            const subtreeLock = await outcome(source.move("blocked"));
            await lock.close();

            const firstMove = source.move(destination, "middle");
            const nameWhilePending = source.name;
            const secondMove = source.move("after");
            const movedNested = source.getDirectoryHandle("nested");
            const resolved = root.resolve(source);
            const [, secondMoveOutcome, nestedAfter, resolvedAfter] = await Promise.all([
                firstMove,
                outcome(secondMove),
                movedNested,
                resolved
            ]);
            const retryAfterSettlement = await outcome(source.move("after"));
            const staleNestedAfterRetry = await outcome(
                nestedAfter.getFileHandle("file.txt"));
            const finalNested = await source.getDirectoryHandle("nested");
            const movedFile = await finalNested.getFileHandle("file.txt");
            const sourceKeys = [];
            for await (const key of root.keys()) sourceKeys.push(key);
            const destinationKeys = [];
            for await (const key of destination.keys()) destinationKeys.push(key);
            postMessage({
                prototype: [
                    typeof FileSystemDirectoryHandle.prototype.move,
                    Object.prototype.hasOwnProperty.call(
                        FileSystemDirectoryHandle.prototype, "move")
                ],
                subtreeLock,
                nameWhilePending,
                finalName: source.name,
                secondMove: secondMoveOutcome,
                retryAfterSettlement,
                staleNestedAfterRetry,
                resolved: resolvedAfter,
                text: await (await movedFile.getFile()).text(),
                sourceKeys,
                destinationKeys
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-directory-move-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"prototype":["function",true],"subtreeLock":"NoModificationAllowedError","nameWhilePending":"source","finalName":"after","secondMove":"NoModificationAllowedError","retryAfterSettlement":"resolved","staleNestedAfterRetry":"NotFoundError","resolved":["destination","middle"],"text":"worker directory move","sourceKeys":["destination"],"destinationKeys":["after"]}"#
    );
}

#[tokio::test]
async fn worker_opfs_owner_state_materializes_on_first_operation() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const sameStorage = navigator.storage === navigator.storage;
            const root = await navigator.storage.getDirectory();
            postMessage({
                sameStorage,
                kind: root.kind,
                directoryConstructorInheritance:
                    Object.getPrototypeOf(FileSystemDirectoryHandle) ===
                        FileSystemHandle &&
                    Object.getPrototypeOf(
                        FileSystemDirectoryHandle.prototype
                    ) === FileSystemHandle.prototype,
                writableConstructorInheritance:
                    Object.getPrototypeOf(FileSystemWritableFileStream) ===
                        WritableStream &&
                    Object.getPrototypeOf(
                        FileSystemWritableFileStream.prototype
                    ) === WritableStream.prototype
            });
        });
        "#
        .into(),
        "https://opfs-owner-state-lazy-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"sameStorage":true,"kind":"directory","directoryConstructorInheritance":true,"writableConstructorInheritance":true}"#
    );
    let diagnostics = timeout(TIMEOUT, handle.resource_owner_slot_diagnostics())
        .await
        .expect("timed out waiting for worker OPFS diagnostics")
        .expect("worker OPFS diagnostics should complete");
    assert!(
        diagnostics.opfs_owner_state_materialized,
        "the first worker OPFS operation must allocate its owner state"
    );
    assert!(
        diagnostics.storage_constructor_materializations >= 2,
        "navigator.storage and the returned directory handle must materialize their constructors"
    );
    assert!(
        diagnostics.storage_manager_materialized,
        "reading navigator.storage must materialize one SameObject wrapper"
    );
    assert!(
        !diagnostics.storage_bucket_manager_materialized,
        "using navigator.storage must not materialize navigator.storageBuckets"
    );
}

#[tokio::test]
async fn worker_opfs_mutation_leases_span_owner_promise_settlement() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const outcome = async promise => {
                try {
                    const value = await promise;
                    return { status: "resolved", value };
                } catch (error) {
                    return { status: error && error.name };
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("mutation-lease.txt", { create: true });

            const movePromise = file.move("moved.txt");
            const writerDuringMove = outcome(
                file.createWritable({ mode: "exclusive" }));
            const [moveResult, moveConflict] = await Promise.all([
                outcome(movePromise),
                writerDuringMove
            ]);
            const writerAfterMove = await file.createWritable({ mode: "exclusive" });
            await writerAfterMove.close();

            const removePromise = file.remove();
            const syncDuringRemove = outcome(
                file.createSyncAccessHandle({ mode: "read-only" }));
            const [removeResult, removeConflict] = await Promise.all([
                outcome(removePromise),
                syncDuringRemove
            ]);
            const syncAfterRemove = await outcome(
                file.createSyncAccessHandle({ mode: "read-only" }));

            postMessage({
                moveResult: moveResult.status,
                moveConflict: moveConflict.status,
                removeResult: removeResult.status,
                removeConflict: removeConflict.status,
                syncAfterRemove: syncAfterRemove.status
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-mutation-lease-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"moveResult":"resolved","moveConflict":"NoModificationAllowedError","removeResult":"resolved","removeConflict":"NoModificationAllowedError","syncAfterRemove":"NotFoundError"}"#
    );
}

#[tokio::test]
async fn worker_teardown_drops_pending_opfs_mutation_completion_and_releases_lock() {
    ensure_v8();
    let bucket_store = crate::new_shared_storage_bucket_store();
    let storage_service = bucket_store.lock().storage_service();
    let script_url = "https://opfs-mutation-teardown.test/worker.js";
    let mut first = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            let mutationFile;
            onmessage = () => {
                const movePromise = mutationFile.move("moved.txt");
                postMessage({
                    phase: "move-dispatched",
                    promise: !!movePromise && typeof movePromise.then === "function"
                });
                close();
            };
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                mutationFile = await root.getFileHandle("before.txt", { create: true });
                postMessage({ phase: "ready" });
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let ready = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for OPFS mutation setup")
        .expect("worker channel closed before mutation setup");
    assert_eq!(expect_post_json(ready), r#"{"phase":"ready"}"#);

    let (blocker_started_tx, blocker_started_rx) = std::sync::mpsc::channel();
    let (release_blocker_tx, release_blocker_rx) = std::sync::mpsc::channel();
    storage_service
        .dispatch_opfs(
            move |_| {
                blocker_started_tx.send(()).unwrap();
                release_blocker_rx.recv().unwrap();
            },
            |_| {},
        )
        .unwrap();
    blocker_started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("storage owner blocker should start");

    first.post_message(serialize_test_string("move"));
    let dispatched = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for pending OPFS move")
        .expect("worker channel closed before move dispatch");
    assert_eq!(
        expect_post_json(dispatched),
        r#"{"phase":"move-dispatched","promise":true}"#
    );
    first.terminate_and_join();

    let (barrier_tx, barrier_rx) = std::sync::mpsc::channel();
    storage_service
        .dispatch_opfs(|_| (), move |result| barrier_tx.send(result).unwrap())
        .unwrap();
    release_blocker_tx
        .send(())
        .expect("storage owner blocker should still be waiting");
    barrier_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("post-mutation storage barrier should finish")
        .expect("post-mutation storage barrier should not panic");

    let mut second = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const moved = await root.getFileHandle("moved.txt");
                const writer = await moved.createWritable({ mode: "exclusive" });
                await writer.write("replacement");
                await writer.close();
                const source = await root.getFileHandle("before.txt").then(
                    () => "present",
                    error => error && error.name
                );
                postMessage({
                    source,
                    text: await (await moved.getFile()).text()
                });
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let replacement = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for post-teardown OPFS writer")
        .expect("replacement worker channel closed");
    assert_eq!(
        expect_post_json(replacement),
        r#"{"source":"NotFoundError","text":"replacement"}"#
    );
}

#[tokio::test]
async fn worker_opfs_writable_acquisition_and_sink_follow_storage_owner_order() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const outcome = async promise => {
                try {
                    await promise;
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("worker-writer.txt", { create: true });

            const acquisition = file.createWritable({ mode: "exclusive" });
            const conflictingMove = file.move("blocked.txt");
            const writer = await acquisition;
            const conflict = await outcome(conflictingMove);

            const first = writer.write("W");
            const second = writer.write(new Uint8Array([88]));
            const third = writer.write({ type: "write", position: 2, data: "Y" });
            const closePromise = writer.close();
            const commandPromises = [first, second, third, closePromise].every(
                value => value && typeof value.then === "function"
            );
            await Promise.all([first, second, third, closePromise]);

            await file.move("after.txt");
            const snapshot = await file.getFile();
            postMessage({
                conflict,
                commandPromises,
                finalName: file.name,
                text: await snapshot.text()
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-writable-owner-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"conflict":"NoModificationAllowedError","commandPromises":true,"finalName":"after.txt","text":"WXY"}"#
    );
}

#[tokio::test]
async fn worker_opfs_sync_access_handle_covers_cursor_flush_lock_and_close() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const rejectedName = async promise => {
                try {
                    await promise;
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const thrownName = callback => {
                try {
                    callback();
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("sync.bin", { create: true });
            const sync = await file.createSyncAccessHandle();
            const shape = [
                sync instanceof FileSystemSyncAccessHandle,
                Object.prototype.toString.call(sync),
                sync.mode,
                ["close", "flush", "getSize", "truncate", "read", "write"]
                    .map(name => typeof sync[name]).join(",")
            ];
            const written = sync.write(new Uint8Array([65, 66, 67]), { at: 2 });
            const sizeAfterWrite = sync.getSize();
            const target = new ArrayBuffer(9);
            new Uint8Array(target).fill(9);
            const view = new Uint8Array(target, 2, 5);
            const read = sync.read(view, { at: 0 });
            const targetBytes = Array.from(new Uint8Array(target));
            sync.truncate(3);
            const sizeAfterTruncate = sync.getSize();
            sync.write(new Uint8Array([90]));
            sync.flush();
            const flushed = Array.from(
                new Uint8Array(await (await file.getFile()).arrayBuffer())
            );
            const lockConflict = await rejectedName(file.createWritable());
            sync.close();
            sync.close();
            const afterClose = [
                thrownName(() => sync.read(new Uint8Array(1))),
                thrownName(() => sync.write(new Uint8Array(1))),
                thrownName(() => sync.flush()),
                thrownName(() => sync.getSize()),
                thrownName(() => sync.truncate(0))
            ];
            const writable = await file.createWritable();
            await writable.write("Q");
            await writable.close();
            const afterUnlock = await new Response(await file.getFile()).text();
            postMessage({
                shape,
                written,
                sizeAfterWrite,
                read,
                targetBytes,
                sizeAfterTruncate,
                flushed,
                lockConflict,
                afterClose,
                afterUnlock
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-sync-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"shape":[true,"[object FileSystemSyncAccessHandle]","readwrite","function,function,function,function,function,function"],"written":3,"sizeAfterWrite":5,"read":5,"targetBytes":[9,9,0,0,65,66,67,9,9],"sizeAfterTruncate":3,"flushed":[0,0,65,90],"lockConflict":"NoModificationAllowedError","afterClose":["InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError","InvalidStateError"],"afterUnlock":"Q"}"#
    );
}

#[tokio::test]
async fn worker_opfs_sync_shared_modes_enforce_compatibility_and_write_permission() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const rejectedName = async promise => {
                try {
                    await promise;
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const thrownName = callback => {
                try {
                    callback();
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("shared-sync.bin", { create: true });
            const initialWriter = await file.createWritable();
            await initialWriter.write("initial");
            await initialWriter.close();

            const readOnlyFirst = await file.createSyncAccessHandle({ mode: "read-only" });
            const readOnlySecond = await file.createSyncAccessHandle({ mode: "read-only" });
            const readOnlyConflicts = [
                await rejectedName(file.createSyncAccessHandle({ mode: "readwrite" })),
                await rejectedName(
                    file.createSyncAccessHandle({ mode: "readwrite-unsafe" })),
                await rejectedName(file.createWritable({ mode: "siloed" }))
            ];
            const readOnlyMutations = [
                thrownName(() => readOnlyFirst.write(new Uint8Array([1]))),
                thrownName(() => readOnlyFirst.truncate(0)),
                thrownName(() => readOnlyFirst.flush())
            ];
            const readOnlyBytes = new Uint8Array(7);
            const readOnlyRead = readOnlyFirst.read(readOnlyBytes, { at: 0 });
            readOnlyFirst.close();
            const oneReadOnlyStillLocks = await rejectedName(
                file.createSyncAccessHandle({ mode: "readwrite-unsafe" }));
            readOnlySecond.close();

            const unsafeFirst = await file.createSyncAccessHandle({
                mode: "readwrite-unsafe"
            });
            const unsafeSecond = await file.createSyncAccessHandle({
                mode: "readwrite-unsafe"
            });
            const unsafeConflicts = [
                await rejectedName(file.createSyncAccessHandle({ mode: "read-only" })),
                await rejectedName(file.createSyncAccessHandle({ mode: "readwrite" })),
                await rejectedName(file.createWritable({ mode: "exclusive" }))
            ];
            unsafeFirst.truncate(0);
            unsafeFirst.write(new TextEncoder().encode("first"));
            const unsafeSeenBytes = new Uint8Array(5);
            const unsafeRead = unsafeSecond.read(unsafeSeenBytes, { at: 0 });
            unsafeSecond.write(new TextEncoder().encode("!"), { at: 5 });
            const beforeFlush = await (await file.getFile()).text();
            unsafeFirst.flush();
            const afterFirstFlush = await (await file.getFile()).text();
            unsafeSecond.flush();
            const afterSecondFlush = await (await file.getFile()).text();
            unsafeFirst.close();
            unsafeSecond.close();

            const afterUnlock = await rejectedName(file.createWritable());
            postMessage({
                modes: [
                    readOnlyFirst.mode,
                    readOnlySecond.mode,
                    unsafeFirst.mode,
                    unsafeSecond.mode
                ],
                readOnlyConflicts,
                readOnlyMutations,
                readOnlyRead,
                readOnlyText: new TextDecoder().decode(readOnlyBytes),
                oneReadOnlyStillLocks,
                unsafeConflicts,
                unsafeRead,
                unsafeSeen: new TextDecoder().decode(unsafeSeenBytes),
                beforeFlush,
                afterFirstFlush,
                afterSecondFlush,
                afterUnlock
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-sync-shared-modes-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"modes":["read-only","read-only","readwrite-unsafe","readwrite-unsafe"],"readOnlyConflicts":["NoModificationAllowedError","NoModificationAllowedError","NoModificationAllowedError"],"readOnlyMutations":["NoModificationAllowedError","NoModificationAllowedError","NoModificationAllowedError"],"readOnlyRead":7,"readOnlyText":"initial","oneReadOnlyStillLocks":"NoModificationAllowedError","unsafeConflicts":["NoModificationAllowedError","NoModificationAllowedError","NoModificationAllowedError"],"unsafeRead":5,"unsafeSeen":"first","beforeFlush":"first!","afterFirstFlush":"first!","afterSecondFlush":"first!","afterUnlock":"resolved"}"#
    );
}

#[tokio::test]
async fn worker_opfs_unsafe_handles_keep_separate_cursors_and_reject_host_file_overflow() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const thrownName = callback => {
                try {
                    callback();
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("unsafe-cursors.bin", { create: true });
            const writer = await file.createWritable();
            await writer.write("abcdef");
            await writer.close();

            const first = await file.createSyncAccessHandle({ mode: "readwrite-unsafe" });
            const second = await file.createSyncAccessHandle({ mode: "readwrite-unsafe" });
            const firstReadBuffer = new Uint8Array(2);
            const secondReadBuffer = new Uint8Array(1);
            const firstRead = first.read(firstReadBuffer, { at: 1 });
            const secondRead = second.read(secondReadBuffer, { at: 5 });
            first.write(new TextEncoder().encode("X"));
            first.truncate(2);
            first.write(new TextEncoder().encode("C"));
            second.write(new TextEncoder().encode("Z"));

            const huge = 2 ** 63;
            const nearLimit = huge - 1024;
            const bounds = [
                thrownName(() => first.read(new Uint8Array(0), { at: huge })),
                thrownName(() => first.write(new Uint8Array(0), { at: huge })),
                thrownName(() => first.write(new Uint8Array(2048), { at: nearLimit })),
                thrownName(() => first.truncate(huge))
            ];
            first.write(new Uint8Array(0), { at: nearLimit });
            bounds.push(thrownName(() => first.write(new Uint8Array(2048))));
            const sizes = [first.getSize(), second.getSize()];
            first.close();
            const firstAfterClose = thrownName(() => first.getSize());
            second.write(new TextEncoder().encode("Q"), { at: 0 });
            const liveBytes = Array.from(
                new Uint8Array(await (await file.getFile()).arrayBuffer())
            );
            second.close();

            postMessage({
                modes: [first.mode, second.mode],
                reads: [firstRead, Array.from(firstReadBuffer),
                        secondRead, Array.from(secondReadBuffer)],
                bounds,
                sizes,
                firstAfterClose,
                liveBytes
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-sync-unsafe-cursors-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"modes":["readwrite-unsafe","readwrite-unsafe"],"reads":[2,[98,99],1,[102]],"bounds":["TypeError","TypeError","QuotaExceededError","TypeError","QuotaExceededError"],"sizes":[7,7],"firstAfterClose":"InvalidStateError","liveBytes":[81,98,67,0,0,0,90]}"#
    );
}

#[tokio::test]
async fn worker_opfs_sync_acquisition_follows_storage_owner_order() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        Promise.resolve().then(async () => {
            const outcome = async promise => {
                try {
                    await promise;
                    return "resolved";
                } catch (error) {
                    return error && error.name;
                }
            };
            const root = await navigator.storage.getDirectory();
            const file = await root.getFileHandle("sync-owner.bin", { create: true });

            const acquisition = file.createSyncAccessHandle();
            const conflictingMove = file.move("blocked.bin");
            const sync = await acquisition;
            const conflict = await outcome(conflictingMove);
            const written = sync.write(new Uint8Array([65, 66, 67]));
            sync.close();

            await file.move("after.bin");
            const snapshot = await file.getFile();
            postMessage({
                conflict,
                mode: sync.mode,
                written,
                finalName: file.name,
                bytes: Array.from(new Uint8Array(await snapshot.arrayBuffer()))
            });
            close();
        }).catch(error => {
            postMessage({ errorName: error && error.name, errorMessage: error && error.message });
            close();
        });
        "#
        .into(),
        "https://opfs-sync-owner-worker.test/worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"conflict":"NoModificationAllowedError","mode":"readwrite","written":3,"finalName":"after.bin","bytes":[65,66,67]}"#
    );
}

#[tokio::test]
async fn worker_teardown_closes_leaked_opfs_sync_handle_and_commits_dirty_data() {
    ensure_v8();
    let bucket_store = crate::new_shared_storage_bucket_store();
    let script_url = "https://opfs-sync-teardown.test/worker.js";
    let mut first = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const file = await root.getFileHandle("leaked-sync.bin", { create: true });
                globalThis.__leakedSync = await file.createSyncAccessHandle();
                globalThis.__leakedSync.write(new Uint8Array([68, 73, 82, 84, 89]));
                postMessage("sync-open");
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let opened = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for leaked OPFS sync handle")
        .expect("worker channel closed before sync handle opened");
    assert_eq!(expect_post_json(opened), r#""sync-open""#);
    first.terminate_and_join();

    let mut second = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const file = await root.getFileHandle("leaked-sync.bin");
                const before = Array.from(
                    new Uint8Array(await (await file.getFile()).arrayBuffer())
                );
                const replacement = await file.createSyncAccessHandle();
                replacement.close();
                postMessage({ before, replacementOpened: true });
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let replacement = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for replacement OPFS sync handle")
        .expect("replacement worker channel closed");
    assert_eq!(
        expect_post_json(replacement),
        r#"{"before":[68,73,82,84,89],"replacementOpened":true}"#
    );
}

#[tokio::test]
async fn worker_context_teardown_aborts_opfs_writable_and_releases_lock() {
    ensure_v8();
    let bucket_store = crate::new_shared_storage_bucket_store();
    let script_url = "https://opfs-writer-teardown.test/worker.js";
    let mut first = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const file = await root.getFileHandle("teardown.txt", { create: true });
                const seed = await file.createWritable();
                await seed.write("committed");
                await seed.close();
                globalThis.__leakedWriter = await file.createWritable({ mode: "exclusive" });
                await globalThis.__leakedWriter.write("must-not-commit");
                postMessage("writer-open");
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let msg = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for open OPFS writer")
        .expect("worker channel closed before writer opened");
    assert_eq!(expect_post_json(msg), r#""writer-open""#);
    first.terminate_and_join();

    let mut second = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const file = await root.getFileHandle("teardown.txt");
                const before = await new Response(await file.getFile()).text();
                const replacement = await file.createWritable({ mode: "exclusive" });
                await replacement.write("replacement");
                await replacement.close();
                const after = await new Response(await file.getFile()).text();
                postMessage({ before, after });
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let msg = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for replacement OPFS writer")
        .expect("worker channel closed before replacement completed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"before":"committed","after":"replacement"}"#
    );
    second.terminate_and_join();
}

#[tokio::test]
async fn worker_teardown_drops_pending_opfs_writable_acquisition_and_releases_lock() {
    ensure_v8();
    let bucket_store = crate::new_shared_storage_bucket_store();
    let storage_service = bucket_store.lock().storage_service();
    let script_url = "https://opfs-acquisition-teardown.test/worker.js";
    let mut first = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            let acquisitionFile;
            onmessage = () => {
                const acquisition = acquisitionFile.createWritable({ mode: "exclusive" });
                postMessage({
                    phase: "acquisition-dispatched",
                    promise: !!acquisition && typeof acquisition.then === "function"
                });
                close();
            };
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                acquisitionFile = await root.getFileHandle(
                    "pending-acquisition.txt",
                    { create: true }
                );
                postMessage({ phase: "ready" });
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let ready = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for OPFS acquisition setup")
        .expect("worker channel closed before acquisition setup");
    assert_eq!(expect_post_json(ready), r#"{"phase":"ready"}"#);

    let (blocker_started_tx, blocker_started_rx) = std::sync::mpsc::channel();
    let (release_blocker_tx, release_blocker_rx) = std::sync::mpsc::channel();
    storage_service
        .dispatch_opfs(
            move |_| {
                blocker_started_tx.send(()).unwrap();
                release_blocker_rx.recv().unwrap();
            },
            |_| {},
        )
        .unwrap();
    blocker_started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("storage owner blocker should start");

    first.post_message(serialize_test_string("acquire"));
    let dispatched = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for pending OPFS acquisition")
        .expect("worker channel closed before acquisition dispatch");
    assert_eq!(
        expect_post_json(dispatched),
        r#"{"phase":"acquisition-dispatched","promise":true}"#
    );
    first.terminate_and_join();
    let (barrier_tx, barrier_rx) = std::sync::mpsc::channel();
    storage_service
        .dispatch_opfs(|_| (), move |result| barrier_tx.send(result).unwrap())
        .unwrap();
    release_blocker_tx
        .send(())
        .expect("storage owner blocker should still be waiting");

    // This queued barrier runs after the acquisition completion has failed to
    // reach the destroyed Worker and dropped its lease. A following
    // synchronous turn is therefore ordered after the lease's abort ticket.
    barrier_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("post-acquisition storage barrier should finish")
        .expect("post-acquisition storage barrier should not panic");
    storage_service.with_opfs(|_| ());

    let mut second = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const file = await root.getFileHandle("pending-acquisition.txt");
                const replacement = await file.createWritable({ mode: "exclusive" });
                await replacement.write("replacement");
                await replacement.close();
                postMessage({ text: await (await file.getFile()).text() });
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let replacement = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for replacement OPFS writer")
        .expect("replacement worker channel closed");
    assert_eq!(expect_post_json(replacement), r#"{"text":"replacement"}"#);
}

#[tokio::test]
async fn worker_teardown_drops_pending_opfs_sync_acquisition_and_releases_lock() {
    ensure_v8();
    let bucket_store = crate::new_shared_storage_bucket_store();
    let storage_service = bucket_store.lock().storage_service();
    let script_url = "https://opfs-sync-acquisition-teardown.test/worker.js";
    let mut first = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            let acquisitionFile;
            onmessage = () => {
                const acquisition = acquisitionFile.createSyncAccessHandle();
                postMessage({
                    phase: "acquisition-dispatched",
                    promise: !!acquisition && typeof acquisition.then === "function"
                });
                close();
            };
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                acquisitionFile = await root.getFileHandle(
                    "pending-sync-acquisition.bin",
                    { create: true }
                );
                postMessage({ phase: "ready" });
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let ready = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for OPFS sync acquisition setup")
        .expect("worker channel closed before sync acquisition setup");
    assert_eq!(expect_post_json(ready), r#"{"phase":"ready"}"#);

    let (blocker_started_tx, blocker_started_rx) = std::sync::mpsc::channel();
    let (release_blocker_tx, release_blocker_rx) = std::sync::mpsc::channel();
    storage_service
        .dispatch_opfs(
            move |_| {
                blocker_started_tx.send(()).unwrap();
                release_blocker_rx.recv().unwrap();
            },
            |_| {},
        )
        .unwrap();
    blocker_started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("storage owner blocker should start");

    first.post_message(serialize_test_string("acquire"));
    let dispatched = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for pending OPFS sync acquisition")
        .expect("worker channel closed before sync acquisition dispatch");
    assert_eq!(
        expect_post_json(dispatched),
        r#"{"phase":"acquisition-dispatched","promise":true}"#
    );
    first.terminate_and_join();

    let (barrier_tx, barrier_rx) = std::sync::mpsc::channel();
    storage_service
        .dispatch_opfs(|_| (), move |result| barrier_tx.send(result).unwrap())
        .unwrap();
    release_blocker_tx
        .send(())
        .expect("storage owner blocker should still be waiting");

    // The acquisition completion drops its lease when delivery to the dead
    // Worker fails. Reserve one more synchronous turn after the intervening
    // barrier so the lease's close ticket has committed before replacement.
    barrier_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("post-sync-acquisition storage barrier should finish")
        .expect("post-sync-acquisition storage barrier should not panic");
    storage_service.with_opfs(|_| ());

    let mut second = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            Promise.resolve().then(async () => {
                const root = await navigator.storage.getDirectory();
                const file = await root.getFileHandle("pending-sync-acquisition.bin");
                const replacement = await file.createSyncAccessHandle();
                const written = replacement.write(new Uint8Array([79, 75]));
                replacement.close();
                const bytes = Array.from(
                    new Uint8Array(await (await file.getFile()).arrayBuffer())
                );
                postMessage({ written, bytes });
                close();
            }).catch(error => {
                postMessage({ errorName: error && error.name, errorMessage: error && error.message });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let replacement = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for replacement OPFS sync handle")
        .expect("replacement worker channel closed");
    assert_eq!(
        expect_post_json(replacement),
        r#"{"written":2,"bytes":[79,75]}"#
    );
}

#[tokio::test]
async fn worker_navigator_uses_loader_identity_for_user_agent_data() {
    ensure_v8();
    const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36";
    let mut config = FetchConfig::default();
    config.set_user_agent(USER_AGENT);
    config.push_default_request_header("Accept-Language", "fr-CA,fr;q=0.8,en;q=0.5");
    let loader = ResourceRequestClient::new(&config).expect("worker loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        const uaData = navigator.userAgentData;
        Promise.all([
            uaData.getHighEntropyValues([]),
            uaData.getHighEntropyValues([
                "architecture",
                "formFactors",
                "fullVersionList",
                "uaFullVersion",
                "unsupported"
            ]),
            uaData.getHighEntropyValues().then(
                () => "resolved",
                error => error && error.name
            )
        ]).then(([empty, selected, missingArgument]) => {
            postMessage({
                userAgent: navigator.userAgent,
                appVersion: navigator.appVersion,
                language: navigator.language,
                languages: Array.from(navigator.languages),
                constructorType: typeof NavigatorUAData,
                constructorOwn: Object.prototype.hasOwnProperty.call(self, "NavigatorUAData"),
                dataType: typeof uaData,
                sameObject: uaData === navigator.userAgentData,
                instance: uaData instanceof NavigatorUAData,
                tag: Object.prototype.toString.call(uaData),
                json: uaData.toJSON(),
                emptyKeys: Object.keys(empty),
                selectedKeys: Object.keys(selected),
                selected,
                missingArgument
            });
            close();
        });
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"userAgent":"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36","appVersion":"5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36","language":"fr-CA","languages":["fr-CA","fr","en"],"constructorType":"function","constructorOwn":true,"dataType":"object","sameObject":true,"instance":true,"tag":"[object NavigatorUAData]","json":{"brands":[{"brand":"Chromium","version":"146"},{"brand":"Not-A.Brand","version":"24"},{"brand":"Google Chrome","version":"146"}],"mobile":false,"platform":"Windows"},"emptyKeys":["brands","mobile","platform"],"selectedKeys":["architecture","brands","formFactors","fullVersionList","mobile","platform","uaFullVersion"],"selected":{"architecture":"x86","brands":[{"brand":"Chromium","version":"146"},{"brand":"Not-A.Brand","version":"24"},{"brand":"Google Chrome","version":"146"}],"formFactors":["Desktop"],"fullVersionList":[{"brand":"Chromium","version":"146.1.2.3"},{"brand":"Not-A.Brand","version":"24.0.0.0"},{"brand":"Google Chrome","version":"146.1.2.3"}],"mobile":false,"platform":"Windows","uaFullVersion":"146.1.2.3"},"missingArgument":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_storage_apis_are_secure_context_only() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const proto = Object.getPrototypeOf(navigator);
        postMessage({
            secure: isSecureContext,
            storageInNavigator: "storage" in navigator,
            storageBucketsInNavigator: "storageBuckets" in navigator,
            serviceWorkerInNavigator: "serviceWorker" in navigator,
            userAgentDataInNavigator: "userAgentData" in navigator,
            storageInProto: Object.prototype.hasOwnProperty.call(proto, "storage"),
            storageBucketsInProto: Object.prototype.hasOwnProperty.call(proto, "storageBuckets"),
            serviceWorkerInProto: Object.prototype.hasOwnProperty.call(proto, "serviceWorker"),
            userAgentDataInProto:
              Object.prototype.hasOwnProperty.call(proto, "userAgentData"),
            storageValueType: typeof navigator.storage,
            storageBucketsValueType: typeof navigator.storageBuckets,
            serviceWorkerValueType: typeof navigator.serviceWorker,
            userAgentDataValueType: typeof navigator.userAgentData,
            storageManagerGlobal: "StorageManager" in self,
            storageEstimateGlobal: "StorageEstimate" in self,
            storageBucketManagerGlobal: "StorageBucketManager" in self,
            storageBucketGlobal: "StorageBucket" in self,
            fileSystemHandleGlobal: "FileSystemHandle" in self,
            fileSystemFileHandleGlobal: "FileSystemFileHandle" in self,
            fileSystemDirectoryHandleGlobal: "FileSystemDirectoryHandle" in self,
            fileSystemWritableFileStreamGlobal:
              "FileSystemWritableFileStream" in self,
            fileSystemSyncAccessHandleGlobal:
              "FileSystemSyncAccessHandle" in self,
        });
        close();
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"secure":false,"storageInNavigator":false,"storageBucketsInNavigator":false,"serviceWorkerInNavigator":false,"userAgentDataInNavigator":false,"storageInProto":false,"storageBucketsInProto":false,"serviceWorkerInProto":false,"userAgentDataInProto":false,"storageValueType":"undefined","storageBucketsValueType":"undefined","serviceWorkerValueType":"undefined","userAgentDataValueType":"undefined","storageManagerGlobal":false,"storageEstimateGlobal":false,"storageBucketManagerGlobal":false,"storageBucketGlobal":false,"fileSystemHandleGlobal":false,"fileSystemFileHandleGlobal":false,"fileSystemDirectoryHandleGlobal":false,"fileSystemWritableFileStreamGlobal":false,"fileSystemSyncAccessHandleGlobal":false}"#
    );
}

#[tokio::test]
async fn worker_cross_origin_isolated_defaults_to_false() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            secure: isSecureContext,
            isolated: crossOriginIsolated,
        });
        close();
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"secure":true,"isolated":false}"#);
}

#[tokio::test]
async fn worker_cross_origin_isolated_reflects_policy_context_capability() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            postMessage({
                secure: isSecureContext,
                isolated: crossOriginIsolated,
            });
            close();
            "#
            .into(),
            "https://example.test/worker/dip.js".into(),
        )
        .with_policy_context(crate::types::SubresourcePolicyContext {
            document_isolation_policy:
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp,
            cross_origin_isolated: true,
            ..Default::default()
        }),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"secure":true,"isolated":true}"#);
}

#[tokio::test]
async fn worker_cross_origin_isolated_does_not_infer_from_dip_policy_context() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            postMessage({
                secure: isSecureContext,
                isolated: crossOriginIsolated,
            });
            close();
            "#
            .into(),
            "https://example.test/worker/dip-only.js".into(),
        )
        .with_policy_context(crate::types::SubresourcePolicyContext {
            document_isolation_policy:
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp,
            ..Default::default()
        }),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#"{"secure":true,"isolated":false}"#);
}

#[tokio::test]
async fn worker_storage_buckets_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        "use strict";
        const manager = navigator.storageBuckets;
        const illegalManagerProbe = StorageBucketManager.prototype.keys.call({}).then(
            () => "resolved",
            error => error && error.name
        );
        const illegalBucketProbe = StorageBucket.prototype.persisted.call({}).then(
            () => "resolved",
            error => error && error.name
        );
        Promise.resolve().then(async () => {
            await manager.open("worker-bucket-b");
            const bucket = await manager.open("worker-bucket-a", {
                durability: "strict",
                quota: 8192,
                persisted: true
            });
            const keysBeforeDelete = await manager.keys();
            const expiresInitial = await bucket.expires();
            const durabilityInitial = await bucket.durability();
            const persistedInitial = await bucket.persisted();
            const expiresDate = Date.now() + 60000;
            await bucket.setExpires(expiresDate);
            const reopened = await manager.open("worker-bucket-a");
            const expiresAfterReopenMatches = await reopened.expires() === expiresDate;
            const durabilityAfterReopen = await reopened.durability();
            const quotaAfterReopen = (await reopened.estimate()).quota;
            const persistedAfterReopen = await reopened.persisted();
            const cache = await reopened.caches.open("worker-cache");
            await cache.put("worker.txt", new Response("worker cache"));
            const cacheMatchText = await (await cache.match("worker.txt")).text();
            const cacheUsageAfterReopen = (await reopened.estimate()).usageDetails.caches > 0;
            await manager.delete("worker-bucket-b");
            const keysAfterDelete = await manager.keys();
            const [illegalManagerError, illegalBucketError] = await Promise.all([
                illegalManagerProbe,
                illegalBucketProbe
            ]);
            postMessage({
                managerCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageBucketManager"),
                bucketCtorOwn: Object.prototype.hasOwnProperty.call(self, "StorageBucket"),
                managerTag: Object.prototype.toString.call(manager),
                managerInstanceof: manager instanceof StorageBucketManager,
                managerSameObject: manager === navigator.storageBuckets,
                managerOwnKeys: Object.prototype.hasOwnProperty.call(manager, "keys"),
                openLength: StorageBucketManager.prototype.open.length,
                keysLength: StorageBucketManager.prototype.keys.length,
                deleteLength: StorageBucketManager.prototype.delete.length,
                bucketTag: Object.prototype.toString.call(bucket),
                bucketInstanceof: bucket instanceof StorageBucket,
                bucketName: bucket.name,
                bucketPersistType: typeof bucket.persist,
                bucketPersistedLength: StorageBucket.prototype.persisted.length,
                bucketEstimateLength: StorageBucket.prototype.estimate.length,
                bucketDurabilityLength: StorageBucket.prototype.durability.length,
                bucketSetExpiresLength: StorageBucket.prototype.setExpires.length,
                bucketExpiresLength: StorageBucket.prototype.expires.length,
                bucketGetDirectoryLength: StorageBucket.prototype.getDirectory.length,
                expiresInitial,
                expiresAfterReopenMatches,
                durabilityInitial,
                durabilityAfterReopen,
                quotaAfterReopen,
                persistedInitial,
                persistedAfterReopen,
                cacheMatchText,
                cacheUsageAfterReopen,
                keysBeforeDelete,
                keysAfterDelete,
                illegalManagerError,
                illegalBucketError
            });
            close();
        }).catch(error => {
            postMessage({
                errorName: error && error.name,
                errorMessage: error && error.message
            });
            close();
        });
        "#
        .into(),
        "http://127.0.0.1/worker/storage-buckets.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"managerCtorOwn":true,"bucketCtorOwn":true,"managerTag":"[object StorageBucketManager]","managerInstanceof":true,"managerSameObject":true,"managerOwnKeys":false,"openLength":1,"keysLength":0,"deleteLength":1,"bucketTag":"[object StorageBucket]","bucketInstanceof":true,"bucketName":"worker-bucket-a","bucketPersistType":"function","bucketPersistedLength":0,"bucketEstimateLength":0,"bucketDurabilityLength":0,"bucketSetExpiresLength":1,"bucketExpiresLength":0,"bucketGetDirectoryLength":0,"expiresInitial":null,"expiresAfterReopenMatches":true,"durabilityInitial":"strict","durabilityAfterReopen":"strict","quotaAfterReopen":8192,"persistedInitial":true,"persistedAfterReopen":true,"cacheMatchText":"worker cache","cacheUsageAfterReopen":true,"keysBeforeDelete":["worker-bucket-a","worker-bucket-b"],"keysAfterDelete":["worker-bucket-a"],"illegalManagerError":"TypeError","illegalBucketError":"TypeError"}"#
    );
}

#[tokio::test]
async fn worker_global_caches_surface_uses_default_bucket() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        "use strict";
        Promise.resolve().then(async () => {
            const cacheStorage = self.caches;
            const sameObject = cacheStorage === self.caches;
            const keysBefore = await cacheStorage.keys();
            const cache = await cacheStorage.open("global-worker-cache");
            await cache.put("worker-global.txt", new Response("global cache"));
            const matched = await cache.match("worker-global.txt");
            const matchedText = await matched.text();
            const storageMatched = await cacheStorage.match("worker-global.txt");
            const storageMatchedText = await storageMatched.text();
            const hasAfterOpen = await cacheStorage.has("global-worker-cache");
            const hasMissing = await cacheStorage.has("missing-cache");
            const keysAfterOpen = await cacheStorage.keys();
            const defaultBucketKeys = await navigator.storageBuckets.keys();
            const deleted = await cacheStorage.delete("global-worker-cache");
            const keysAfterDelete = await cacheStorage.keys();
            await navigator.storageBuckets.delete("default");
            postMessage({
                hasOwnCaches: Object.prototype.hasOwnProperty.call(self, "caches"),
                cacheStorageTag: Object.prototype.toString.call(cacheStorage),
                sameObject,
                keysBefore,
                cacheTag: Object.prototype.toString.call(cache),
                matchedStatus: matched.status,
                matchedText,
                storageMatchedText,
                hasAfterOpen,
                hasMissing,
                keysAfterOpen,
                defaultBucketKeys,
                deleted,
                keysAfterDelete
            });
            close();
        }).catch(error => {
            postMessage({
                errorName: error && error.name,
                errorMessage: error && error.message
            });
            close();
        });
        "#
        .into(),
        "https://example.test/worker/global-caches.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"hasOwnCaches":true,"cacheStorageTag":"[object CacheStorage]","sameObject":true,"keysBefore":[],"cacheTag":"[object Cache]","matchedStatus":200,"matchedText":"global cache","storageMatchedText":"global cache","hasAfterOpen":true,"hasMissing":false,"keysAfterOpen":["global-worker-cache"],"defaultBucketKeys":[],"deleted":true,"keysAfterDelete":[]}"#
    );
}

#[tokio::test]
async fn worker_location_surface_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const beforeHref = location.href;
        location.href = "https://example.com/ignored";
        const proto = Object.getPrototypeOf(location);
        const href = Object.getOwnPropertyDescriptor(proto, "href");
        const toString = Object.getOwnPropertyDescriptor(proto, "toString");
        postMessage({
            locationOwn: Object.prototype.hasOwnProperty.call(self, "location"),
            ctorOwn: Object.prototype.hasOwnProperty.call(self, "WorkerLocation"),
            ctorType: typeof WorkerLocation,
            ctorName: location.constructor && location.constructor.name,
            protoCtor: proto && proto.constructor && proto.constructor.name,
            tag: Object.prototype.toString.call(location),
            stringified: String(location),
            directToString: toString.value.call(location),
            instanceofWorkerLocation: location instanceof WorkerLocation,
            ownHref: Object.prototype.hasOwnProperty.call(location, "href"),
            hrefGetterType: typeof href?.get,
            toStringDescriptor: [
                typeof toString?.value,
                toString?.value?.name,
                toString?.value?.length,
                toString?.enumerable,
                toString?.writable,
                toString?.configurable
            ].join(":"),
            href: location.href,
            origin: location.origin,
            protocol: location.protocol,
            host: location.host,
            hostname: location.hostname,
            port: location.port,
            pathname: location.pathname,
            search: location.search,
            hash: location.hash,
            unchanged: location.href === beforeHref,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?srch%20#hash".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"locationOwn":true,"ctorOwn":true,"ctorType":"function","ctorName":"WorkerLocation","protoCtor":"WorkerLocation","tag":"[object WorkerLocation]","stringified":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","directToString":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","instanceofWorkerLocation":true,"ownHref":false,"hrefGetterType":"function","toStringDescriptor":"function:toString:0:true:true:true","href":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","origin":"http://127.0.0.1:38080","protocol":"http:","host":"127.0.0.1:38080","hostname":"127.0.0.1","port":"38080","pathname":"/worker/main.js","search":"?srch%20","hash":"#hash","unchanged":true}"##
    );
}

#[tokio::test]
async fn worker_location_backing_slot_ignores_reflection_and_spoofing() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const internal = "__moliWorkerLocationData";
        const beforeHref = location.href;
        const beforeStringified = String(location);
        const proto = Object.getPrototypeOf(location);
        const hrefGetter = Object.getOwnPropertyDescriptor(proto, "href").get;
        const toString = Object.getOwnPropertyDescriptor(proto, "toString").value;
        const reflectedBefore = Object.getOwnPropertyNames(location).includes(internal);
        const fakeBacking = {
            href: "https://spoofed.example/worker.js",
            origin: "https://spoofed.example",
            protocol: "https:",
            host: "spoofed.example",
            hostname: "spoofed.example",
            port: "",
            pathname: "/worker.js",
            search: "",
            hash: ""
        };
        Object.defineProperty(location, internal, {
            value: fakeBacking,
            configurable: true
        });
        const fakeReceiver = {};
        Object.defineProperty(fakeReceiver, internal, {
            value: fakeBacking,
            configurable: true
        });
        postMessage({
            reflectedBefore,
            ownAfterSpoof: Object.prototype.hasOwnProperty.call(location, internal),
            hrefAfterSpoof: location.href,
            stringAfterSpoof: String(location),
            getterCallAfterSpoof: hrefGetter.call(location),
            fakeHrefIsUndefined: hrefGetter.call(fakeReceiver) === undefined,
            fakeStringIsUndefined: toString.call(fakeReceiver) === undefined,
            unchanged: location.href === beforeHref && String(location) === beforeStringified,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?srch%20#hash".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"reflectedBefore":false,"ownAfterSpoof":true,"hrefAfterSpoof":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","stringAfterSpoof":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","getterCallAfterSpoof":"http://127.0.0.1:38080/worker/main.js?srch%20#hash","fakeHrefIsUndefined":true,"fakeStringIsUndefined":true,"unchanged":true}"##
    );
}

#[tokio::test]
async fn worker_script_url_slot_ignores_global_reflection_and_spoofing() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const internal = "__moliWorkerCurrentScriptUrl";
        const reflectedBefore = Object.getOwnPropertyNames(globalThis).includes(internal);
        const requestBefore = new Request("./api").url;
        Object.prototype[internal] = "https://prototype-spoof.example/proto.js";
        globalThis[internal] = "https://own-spoof.example/own.js";
        const requestAfter = new Request("./api").url;
        postMessage({
            reflectedBefore,
            ownAfterSpoof: Object.prototype.hasOwnProperty.call(globalThis, internal),
            publicSpoof: globalThis[internal],
            requestBefore,
            requestAfter,
            stable: requestBefore === requestAfter,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?srch%20#hash".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"reflectedBefore":false,"ownAfterSpoof":true,"publicSpoof":"https://own-spoof.example/own.js","requestBefore":"http://127.0.0.1:38080/worker/api","requestAfter":"http://127.0.0.1:38080/worker/api","stable":true}"##
    );
}

#[tokio::test]
async fn worker_location_data_url_uses_null_origin() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            href: location.href,
            origin: location.origin,
            protocol: location.protocol,
            host: location.host,
            pathname: location.pathname,
            search: location.search,
            hash: location.hash,
            stringified: location.toString(),
        });
        close();
        "#
        .into(),
        "data:text/javascript,hello#frag".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"href":"data:text/javascript,hello#frag","origin":"null","protocol":"data:","host":"","pathname":"text/javascript,hello","search":"","hash":"#frag","stringified":"data:text/javascript,hello#frag"}"##
    );
}

#[tokio::test]
async fn worker_location_empty_query_serializes_to_empty_search() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            href: location.href,
            search: location.search,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js?".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"href":"http://127.0.0.1:38080/worker/main.js?","search":""}"##
    );
}

#[tokio::test]
async fn worker_location_empty_fragment_serializes_to_empty_hash() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage({
            href: location.href,
            hash: location.hash,
        });
        close();
        "#
        .into(),
        "http://127.0.0.1:38080/worker/main.js#".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r##"{"href":"http://127.0.0.1:38080/worker/main.js#","hash":""}"##
    );
}

#[tokio::test]
async fn data_url_worker_exposes_indexeddb_but_denies_opaque_origin_access() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let openError = null;
        try {
            indexedDB.open("opaque-worker-db");
        } catch (error) {
            openError = error.name;
        }
        indexedDB.databases().then(
          () => "resolved",
          error => error && error.name
        ).then(databasesError => postMessage({
            present: "indexedDB" in self,
            type: typeof indexedDB,
            openType: typeof indexedDB.open,
            databasesType: typeof indexedDB.databases,
            openError,
            databasesError
        })).finally(() => close());
        "#
        .into(),
        "data:text/javascript,hello".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"present":true,"type":"object","openType":"function","databasesType":"function","openError":"SecurityError","databasesError":"SecurityError"}"#
    );
}

#[tokio::test]
async fn data_url_worker_storage_bucket_api_is_hidden_in_insecure_context() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const proto = Object.getPrototypeOf(navigator);
        postMessage({
            secure: isSecureContext,
            storageInNavigator: "storage" in navigator,
            storageBucketsInNavigator: "storageBuckets" in navigator,
            storageInProto: Object.prototype.hasOwnProperty.call(proto, "storage"),
            storageBucketsInProto: Object.prototype.hasOwnProperty.call(proto, "storageBuckets"),
            storageValueType: typeof navigator.storage,
            storageBucketsValueType: typeof navigator.storageBuckets,
            storageManagerGlobal: "StorageManager" in self,
            storageEstimateGlobal: "StorageEstimate" in self,
            storageBucketManagerGlobal: "StorageBucketManager" in self,
            storageBucketGlobal: "StorageBucket" in self,
            fileSystemHandleGlobal: "FileSystemHandle" in self,
            fileSystemFileHandleGlobal: "FileSystemFileHandle" in self,
            fileSystemDirectoryHandleGlobal: "FileSystemDirectoryHandle" in self,
            fileSystemWritableFileStreamGlobal:
              "FileSystemWritableFileStream" in self,
            fileSystemSyncAccessHandleGlobal:
              "FileSystemSyncAccessHandle" in self,
        });
        close();
        "#
        .into(),
        "data:text/javascript,storage-buckets".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"secure":false,"storageInNavigator":false,"storageBucketsInNavigator":false,"storageInProto":false,"storageBucketsInProto":false,"storageValueType":"undefined","storageBucketsValueType":"undefined","storageManagerGlobal":false,"storageEstimateGlobal":false,"storageBucketManagerGlobal":false,"storageBucketGlobal":false,"fileSystemHandleGlobal":false,"fileSystemFileHandleGlobal":false,"fileSystemDirectoryHandleGlobal":false,"fileSystemWritableFileStreamGlobal":false,"fileSystemSyncAccessHandleGlobal":false}"#
    );
}

#[tokio::test]
async fn third_party_worker_storage_uses_partitioned_storage_key() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let bucket_store = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
    let script_url = "https://worker.example/partitioned-worker-storage.js";
    let top_level_site = "https://app.example";
    let storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &url::Url::parse(script_url).expect("worker script URL should parse"),
        top_level_site.to_owned(),
        None,
    );
    let serialized_storage_key = storage_key.serialized_storage_key();

    let db_name = "partitioned-worker-idb";
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const bucket = await navigator.storageBuckets.open("partitioned-worker-bucket");
                const keys = await navigator.storageBuckets.keys();
                const open = indexedDB.open("partitioned-worker-idb", 7);
                open.onupgradeneeded = () => {
                    open.result.createObjectStore("kv");
                };
                open.onerror = () => {
                    postMessage({ stage: "open", name: open.error && open.error.name });
                    close();
                };
                open.onsuccess = () => {
                    const db = open.result;
                    const tx = db.transaction("kv", "readwrite");
                    tx.objectStore("kv").put("stored", "key");
                    tx.onerror = () => {
                        postMessage({ stage: "tx", name: tx.error && tx.error.name });
                        db.close();
                        close();
                    };
                    tx.oncomplete = () => {
                        db.close();
                        postMessage({
                            bucketName: bucket.name,
                            keys,
                            idbOpen: true
                        });
                        close();
                    };
                };
            })().catch(error => {
                postMessage({ stage: "promise", name: error && error.name });
                close();
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_storage_key_top_level_site(Some(top_level_site.to_owned()))
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)))
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"bucketName":"partitioned-worker-bucket","keys":["partitioned-worker-bucket"],"idbOpen":true}"#
    );

    assert_eq!(
        manager
            .lock()
            .database_version(&serialized_storage_key, db_name)
            .expect("partitioned IndexedDB version should be readable"),
        Some(7)
    );
    assert_eq!(
        manager
            .lock()
            .database_version("https://worker.example", db_name)
            .expect("script-origin IndexedDB version should be readable"),
        None
    );
    assert_eq!(
        bucket_store.lock().keys(&serialized_storage_key),
        vec!["partitioned-worker-bucket".to_owned()]
    );
    assert_eq!(
        bucket_store.lock().keys("https://worker.example"),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn service_worker_storage_uses_explicit_registration_storage_key() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let bucket_store = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
    let script_url = "https://cdn.example/service-worker-storage.js";
    let storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
        &url::Url::parse(script_url).expect("worker script URL should parse"),
        "https://app.example".to_owned(),
        None,
    );
    let serialized_storage_key = storage_key.serialized_storage_key();

    let db_name = "service-worker-idb";
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const bucket = await navigator.storageBuckets.open("service-worker-bucket");
                const open = indexedDB.open("service-worker-idb", 11);
                open.onupgradeneeded = () => {
                    open.result.createObjectStore("kv");
                };
                open.onerror = () => {
                    throw open.error;
                };
                open.onsuccess = () => {
                    const db = open.result;
                    const tx = db.transaction("kv", "readwrite");
                    tx.objectStore("kv").put("stored", "key");
                    tx.onerror = () => {
                        db.close();
                        throw tx.error;
                    };
                    tx.oncomplete = () => {
                        db.close();
                        if (bucket.name !== "service-worker-bucket") {
                            throw new Error("unexpected bucket name");
                        }
                        skipWaiting();
                    };
                };
            })().catch(error => {
                throw error;
            });
            "#
            .into(),
            script_url.to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://cdn.example/").unwrap(),
        })
        .with_api_storage_key(Some(storage_key.clone()))
        .with_broadcast_channel_top_level_site(Some("https://ignored.example".to_owned()))
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)))
        .with_storage_bucket_store(Some(bucket_store.clone())),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::ServiceWorkerSkipWaiting {
            registration_id,
            version_id,
        } => {
            assert_eq!(
                registration_id,
                crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1)
            );
            assert_eq!(
                version_id,
                crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1)
            );
        }
        other => panic!("expected service worker skipWaiting, got {other:?}"),
    }

    assert_eq!(
        manager
            .lock()
            .database_version(&serialized_storage_key, db_name)
            .expect("registration-key IndexedDB version should be readable"),
        Some(11)
    );
    assert_eq!(
        manager
            .lock()
            .database_version("https://cdn.example", db_name)
            .expect("script-origin IndexedDB version should be readable"),
        None
    );
    assert_eq!(
        bucket_store.lock().keys(&serialized_storage_key),
        vec!["service-worker-bucket".to_owned()]
    );
    assert_eq!(
        bucket_store.lock().keys("https://cdn.example"),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn worker_deleted_storage_bucket_handle_rejects_metadata_and_indexeddb() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let bucket_store = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const bucket = await navigator.storageBuckets.open("deleted-worker-bucket", {
                    durability: "strict"
                });
                await navigator.storageBuckets.delete("deleted-worker-bucket");
                const outcome = promise => promise.then(
                    () => "fulfilled",
                    error => error && error.name
                );
                const idb = await new Promise(resolve => {
                    const request = bucket.indexedDB.open("messages");
                    request.onsuccess = () => {
                        request.result.close();
                        resolve("fulfilled");
                    };
                    request.onerror = () => resolve(request.error && request.error.name);
                });
                postMessage({
                    persisted: await outcome(bucket.persisted()),
                    durability: await outcome(bucket.durability()),
                    expires: await outcome(bucket.expires()),
                    setExpires: await outcome(bucket.setExpires(Date.now() + 1000)),
                    idb
                });
                close();
            })().catch(error => {
                postMessage({ stage: "promise", name: error && error.name });
                close();
            });
            "#
            .into(),
            "https://worker.example/deleted-storage-bucket.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)))
        .with_storage_bucket_store(Some(bucket_store)),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"persisted":"UnknownError","durability":"UnknownError","expires":"UnknownError","setExpires":"UnknownError","idb":"UnknownError"}"#
    );
}

#[tokio::test]
async fn worker_manager_only_partition_binds_named_bucket_indexeddb_quota_owner() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
                const request = request => new Promise((resolve, reject) => {
                    request.onsuccess = () => resolve(request.result);
                    request.onerror = () => reject(request.error);
                });
                const transaction = transaction => new Promise((resolve, reject) => {
                    transaction.oncomplete = () => resolve();
                    transaction.onabort = transaction.onerror = () => reject(transaction.error);
                });
                const bucket = await navigator.storageBuckets.open("manager-only", {
                    quota: 10000
                });
                const open = bucket.indexedDB.open("quota", 1);
                open.onupgradeneeded = () => open.result.createObjectStore("kv");
                const db = await request(open);
                const tx = db.transaction("kv", "readwrite");
                const committed = transaction(tx);
                tx.objectStore("kv").put(new Uint8Array(5000), "stored");
                await committed;
                const usage = await bucket.estimate();
                const root = await bucket.getDirectory();
                const file = await root.getFileHandle("blocked.bin", { create: true });
                const writer = await file.createWritable();
                let blocked;
                try {
                    await writer.write(new Uint8Array(6000));
                    blocked = "resolved";
                } catch (error) {
                    blocked = error && error.name;
                }
                db.close();
                await navigator.storageBuckets.delete("manager-only");
                postMessage({
                    indexedDbUsagePositive: usage.usageDetails.indexedDB > 0,
                    blocked
                });
                close();
            })().catch(error => {
                postMessage({ stage: "promise", name: error && error.name });
                close();
            });
            "#
            .into(),
            "https://worker.example/manager-only-bucket-owner.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"indexedDbUsagePositive":true,"blocked":"QuotaExceededError"}"#
    );
}

#[tokio::test]
async fn worker_indexed_db_request_chain_does_not_starve_timers() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const open = indexedDB.open("worker-idb-timer-fairness-" + Math.random(), 1);
            open.onupgradeneeded = () => {
                open.result.createObjectStore("store");
            };
            open.onerror = () => {
                postMessage({ stage: "open", name: open.error && open.error.name });
                close();
            };
            open.onsuccess = () => {
                const db = open.result;
                const tx = db.transaction("store", "readonly");
                let keepSpinning = true;
                let spins = 0;
                function finish(status) {
                    keepSpinning = false;
                    db.close();
                    postMessage(status);
                    close();
                }
                function spin() {
                    if (!keepSpinning) {
                        return;
                    }
                    spins += 1;
                    if (spins > 1000) {
                        finish("timer-starved");
                        return;
                    }
                    tx.objectStore("store").get(0).onsuccess = spin;
                }
                setTimeout(() => finish("timer-fired"), 0);
                spin();
            };
            "#
            .into(),
            "https://worker.example/indexeddb-timer-fairness.js".to_owned(),
        )
        .with_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager))),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""timer-fired""#);
}

#[tokio::test]
async fn http_worker_indexeddb_uses_storage_manager_and_rejects_wasm_module_storage() {
    ensure_v8();
    let manager =
        crate::new_indexed_db_manager(None).expect("in-memory indexedDB manager should initialize");
    let mut handle =
        crate::worker::thread::spawn_worker_with_request_client_and_kind_network_policy_and_broadcast_channel_registry(
            r#"
            const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
            const dbName = "worker-wasm-module-storage-" + Math.random();
            const open = indexedDB.open(dbName, 1);
            open.onupgradeneeded = () => {
                open.result.createObjectStore("store");
            };
            open.onerror = () => {
                postMessage({ stage: "open", name: open.error && open.error.name });
                close();
            };
            open.onsuccess = () => {
                const tx = open.result.transaction("store", "readwrite");
                const store = tx.objectStore("store");
                let thrown = null;
                try {
                    const request = store.put(new WebAssembly.Module(bytes), "module");
                    request.onerror = () => {
                        postMessage({ stage: "put", name: request.error && request.error.name });
                        close();
                    };
                    request.onsuccess = () => {
                        postMessage({ stage: "put", name: "unexpected-success" });
                        close();
                    };
                } catch (error) {
                    thrown = error.name;
                    postMessage({ stage: "put", name: thrown });
                    close();
                }
            };
            "#
            .into(),
            "https://worker.example/indexeddb.js".into(),
            worker_test_request_client(),
            WorkerScriptKind::Classic,
            crate::worker::handle::WorkerNetworkPolicy::default(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            None,
            Some(crate::downgrade_indexed_db_manager(&manager)),
        );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"stage":"put","name":"DataCloneError"}"#
    );
}

#[tokio::test]
async fn worker_filelist_interface_object_is_available() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const file = new File(["hello"], "note.txt", { type: "text/plain" });
        const list = new FileList([file]);
        postMessage({
            ctorOwn: Object.prototype.hasOwnProperty.call(self, "FileList"),
            ctorType: typeof FileList,
            ctorName: list.constructor && list.constructor.name,
            tag: Object.prototype.toString.call(list),
            instanceofFileList: list instanceof FileList,
            length: list.length,
            firstName: list.item(0) && list.item(0).name,
            indexName: list[0] && list[0].name,
            iterType: typeof list[Symbol.iterator],
            iterName: Array.from(list).map(file => file.name).join(","),
        });
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"ctorOwn":true,"ctorType":"function","ctorName":"FileList","tag":"[object FileList]","instanceofFileList":true,"length":1,"firstName":"note.txt","indexName":"note.txt","iterType":"function","iterName":"note.txt"}"#
    );
}

#[tokio::test]
async fn worker_blob_surface_supports_response_blob_and_blob_url_import_scripts() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            const response = new Response("hello from worker blob", {
                headers: { "Content-Type": "text/plain;charset=utf-8" }
            });
            const blob = await response.blob();
            const blobUrl = URL.createObjectURL(new Blob([
                "postMessage({ imported: true, objectUrlType: typeof URL.createObjectURL });"
            ], { type: "text/javascript" }));
            postMessage({
                blobCtor: typeof Blob,
                blobTag: Object.prototype.toString.call(blob),
                size: blob.size,
                type: blob.type,
                text: await blob.text(),
                blobUrlPrefix: blobUrl.startsWith("blob:http://127.0.0.1/"),
            });
            importScripts(blobUrl);
            URL.revokeObjectURL(blobUrl);
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let first = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(first),
        r#"{"blobCtor":"function","blobTag":"[object Blob]","size":22,"type":"text/plain;charset=utf-8","text":"hello from worker blob","blobUrlPrefix":true}"#
    );

    let second = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(second),
        r#"{"imported":true,"objectUrlType":"function"}"#
    );
}

#[tokio::test]
async fn worker_file_and_filereader_surface_reads_file_asynchronously() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            const file = new File(["hello worker file"], "note.txt", {
                type: "text/plain",
                lastModified: 123
            });
            const fileText = await file.text();
            const reader = new FileReader();
            const events = [];
            reader.onloadstart = () => events.push("loadstart");
            reader.addEventListener("progress", () => events.push("progress"));
            reader.onload = () => {
                events.push(`load:${reader.result}`);
            };
            reader.onloadend = () => {
                events.push(`loadend:${reader.readyState}`);
                postMessage({
                    fileCtor: typeof File,
                    readerCtor: typeof FileReader,
                    fileTag: Object.prototype.toString.call(file),
                    fileInstanceofBlob: file instanceof Blob,
                    readerInstanceofEventTarget: reader instanceof EventTarget,
                    fileName: file.name,
                    fileLastModified: file.lastModified,
                    fileType: file.type,
                    fileText,
                    constants: [FileReader.EMPTY, FileReader.LOADING, FileReader.DONE],
                    events,
                });
                close();
            };
            events.push(`before:${reader.readyState}`);
            reader.readAsText(file);
            events.push(`after:${reader.readyState}:${reader.result === null}`);
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"fileCtor":"function","readerCtor":"function","fileTag":"[object File]","fileInstanceofBlob":true,"readerInstanceofEventTarget":true,"fileName":"note.txt","fileLastModified":123,"fileType":"text/plain","fileText":"hello worker file","constants":[0,1,2],"events":["before:0","after:1:true","loadstart","progress","load:hello worker file","loadend:2"]}"#
    );
}

#[tokio::test]
async fn worker_filereader_abort_suppresses_late_load_from_pending_queue() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const reader = new FileReader();
            const events = [];
            reader.onload = () => events.push("load");
            reader.onabort = () => events.push("abort");
            reader.onloadend = () => events.push("loadend");
            reader.readAsText(new File(["cancel"], "cancel.txt"));
            reader.abort();
            setTimeout(() => {
                postMessage({
                    readerReadyState: reader.readyState,
                    resultIsNull: reader.result === null,
                    events,
                });
                close();
            }, 0);
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readerReadyState":2,"resultIsNull":true,"events":["abort","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_does_not_expose_response_xml() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const xhr = new XMLHttpRequest();
        postMessage({
            instanceHasResponseXml: "responseXML" in xhr,
            prototypeHasResponseXml: Object.prototype.hasOwnProperty.call(
                XMLHttpRequest.prototype,
                "responseXML"
            ),
        });
        close();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"instanceHasResponseXml":false,"prototypeHasResponseXml":false}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_uses_worker_script_base_url_and_event_target_listeners() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "hello from worker xhr".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onreadystatechange = (event) => {
                events.push(`prop-rs:${event.type}:${xhr.readyState}`);
            };
            xhr.onload = () => {
                events.push(`prop-load:${xhr.status}`);
            };
            xhr.onloadend = () => {
                events.push(`prop-loadend:${xhr.readyState}`);
            };
            xhr.addEventListener('readystatechange', (event) => {
                events.push(`rs:${event.type}:${xhr.readyState}`);
            });
            xhr.addEventListener('load', () => {
                events.push(`load:${xhr.status}:${xhr.responseText}`);
            });
            xhr.addEventListener('loadend', () => {
                events.push(`loadend:${xhr.readyState}`);
            });
            xhr.addEventListener('loadend', () => {
                postMessage({
                    ctor: typeof XMLHttpRequest,
                    eventTarget: xhr instanceof XMLHttpRequestEventTarget,
                    uploadTag: Object.prototype.toString.call(xhr.upload),
                    status: xhr.status,
                    url: xhr.responseURL,
                    text: xhr.responseText,
                    readyState: xhr.readyState,
                    events,
                });
                close();
            });
            xhr.open('GET', './data.txt');
            xhr.send();
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"ctor":"function","eventTarget":true,"uploadTag":"[object XMLHttpRequestUpload]","status":200,"url":"{base_url}/assets/data.txt","text":"hello from worker xhr","readyState":4,"events":["prop-rs:readystatechange:1","rs:readystatechange:1","prop-rs:readystatechange:2","rs:readystatechange:2","prop-rs:readystatechange:3","rs:readystatechange:3","prop-rs:readystatechange:4","rs:readystatechange:4","prop-load:200","load:200:hello from worker xhr","prop-loadend:4","loadend:4"]}}"#
        )
    );
    server.await.expect("worker xhr server should finish");
}

#[tokio::test]
async fn worker_xhr_request_stage_interception_can_fulfill_synthetic_response() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            xhr.onloadend = () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    header: xhr.getResponseHeader("x-worker-xhr-intercept"),
                    text: xhr.responseText,
                });
                close();
            };
            xhr.open("POST", "http://example.test/intercepted-worker-xhr");
            xhr.send("payload");
        };
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader"),
        WorkerNetworkPolicy {
            network_partition_key: Some("credentialless-worker-xhr".to_owned()),
            ..WorkerNetworkPolicy::default()
        },
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr pause, got {pending:?}");
    };
    assert!(pending.info.network_request_handle.is_none());
    assert_eq!(pending.info.resource_type, SubresourceResourceType::Xhr);
    assert_eq!(
        pending.info.url.as_str(),
        "http://example.test/intercepted-worker-xhr"
    );
    assert_eq!(pending.info.request_body.as_deref(), Some("payload"));
    assert_eq!(
        pending.network_partition_key.as_deref(),
        Some("credentialless-worker-xhr")
    );

    let request = pending_worker_xhr_continue(pending.fetch_id, 31, &pending.info, false);
    handle.fulfill_pending_xhr(
        request,
        204,
        vec![
            ("content-type".to_owned(), "text/plain".to_owned()),
            (
                "x-worker-xhr-intercept".to_owned(),
                "request-stage".to_owned(),
            ),
        ],
        RendererSyntheticResponseBody::from_bytes(b"fulfilled-worker-xhr".to_vec()),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"readyState":4,"status":204,"header":"request-stage","text":"fulfilled-worker-xhr"}"#
    );
}

#[tokio::test]
async fn worker_sync_xhr_request_stage_interception_reports_explicit_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener("readystatechange", () => events.push("readystatechange:" + xhr.readyState));
            xhr.addEventListener("loadstart", () => events.push("loadstart"));
            xhr.addEventListener("error", () => events.push("error"));
            xhr.addEventListener("loadend", () => events.push("loadend"));
            xhr.open("GET", "http://example.test/sync-worker-xhr-intercepted", false);
            let error = null;
            try {
                xhr.send();
            } catch (caught) {
                error = {
                    name: caught && caught.name,
                    message: caught && caught.message,
                    isDomException: caught instanceof DOMException,
                };
            }
            postMessage({
                error,
                readyState: xhr.readyState,
                status: xhr.status,
                responseText: xhr.responseText,
                events,
            });
            close();
        };
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader"),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("worker channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            WorkerToParentMessage::PendingSubresourceFetch(pending) => {
                panic!("sync worker XHR should not be paused for interception: {pending:?}")
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("sync worker XHR interception should record network failure");
    assert_eq!(
        record.url().as_str(),
        "http://example.test/sync-worker-xhr-intercepted"
    );
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "Synchronous XMLHttpRequest interception is not supported"
    ));
    assert_eq!(
        post.expect("sync worker XHR interception should post final surface"),
        r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'http://example.test/sync-worker-xhr-intercepted'.","isDomException":true},"readyState":4,"status":0,"responseText":"","events":["readystatechange:1"]}"#
    );
}

#[tokio::test]
async fn worker_xhr_response_stage_interception_pauses_before_done() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/xhr.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "origin-worker-xhr".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const readyStates = [];
            xhr.onreadystatechange = () => readyStates.push(xhr.readyState);
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    header: xhr.getResponseHeader("x-worker-xhr-response-stage"),
                    text: xhr.responseText,
                    readyStates,
                });
                close();
            };
            xhr.open("GET", "./xhr.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr pause, got {pending:?}");
    };
    let request = pending_worker_xhr_continue(pending.fetch_id, 37, &pending.info, true);
    handle.continue_pending_xhr(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected worker xhr response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 37);
    assert_eq!(info.resource_type, SubresourceResourceType::Xhr);
    assert_eq!(info.response_status, 200);
    assert_eq!(info.response_body.text().as_ref(), "origin-worker-xhr");

    handle.continue_pending_xhr_response(
        request,
        Some(206),
        Some(vec![
            ("content-type".to_owned(), "text/plain".to_owned()),
            (
                "x-worker-xhr-response-stage".to_owned(),
                "continued".to_owned(),
            ),
        ]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":206,"header":"continued","text":"origin-worker-xhr","readyStates":[1,2,3,4]}"#
    );
    server
        .await
        .expect("worker xhr response-stage server should finish");
}

#[tokio::test]
async fn worker_sync_xhr_timeout_cancels_fetch_and_throws_without_progress_events() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind slow sync worker xhr server");
    let addr = listener.local_addr().expect("slow sync worker xhr addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept slow sync worker xhr request");
        read_http_request_head(&mut stream)
            .await
            .expect("read slow sync worker xhr request");
        tokio::time::sleep(Duration::from_millis(250)).await;
        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nslow";
        let _ = stream.write_all(response.as_bytes()).await;
    });
    let base_url = format!("http://{addr}");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener("readystatechange", () => events.push("readystatechange:" + xhr.readyState));
            xhr.addEventListener("loadstart", () => events.push("loadstart"));
            xhr.addEventListener("timeout", () => events.push("timeout:" + xhr.readyState + ":" + xhr.status));
            xhr.addEventListener("error", () => events.push("error"));
            xhr.addEventListener("load", () => events.push("load"));
            xhr.addEventListener("loadend", () => events.push("loadend:" + xhr.readyState + ":" + xhr.status));
            xhr.timeout = 30;
            xhr.open("GET", "./slow.txt", false);
            let error = null;
            try {
                xhr.send();
            } catch (caught) {
                error = {
                    name: caught && caught.name,
                    message: caught && caught.message,
                    isDomException: caught instanceof DOMException,
                };
            }
            postMessage({
                error,
                readyState: xhr.readyState,
                status: xhr.status,
                responseText: xhr.responseText,
                events,
            });
            close();
        })();
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("worker channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("sync worker XHR timeout should record network failure");
    assert_eq!(record.url().as_str(), format!("{base_url}/worker/slow.txt"));
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    let outcome = record.outcome();
    assert!(
        matches!(
            outcome,
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text == "Synchronous XMLHttpRequest timed out after 30 ms"
        ),
        "expected sync worker XHR timeout failure, got {outcome:?}"
    );
    assert_eq!(
        post.expect("sync worker XHR timeout should post final surface"),
        format!(
            r#"{{"error":{{"name":"TimeoutError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load '{base_url}/worker/slow.txt'.","isDomException":true}},"readyState":4,"status":0,"responseText":"","events":["readystatechange:1"]}}"#
        )
    );
    server
        .await
        .expect("slow sync worker xhr server should finish");
}

#[tokio::test]
async fn worker_sync_xhr_allows_response_types_and_omits_progress_events() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
          const probe = (type, setBeforeOpen) => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onreadystatechange = () => events.push(`readystatechange:${xhr.readyState}`);
            for (const eventType of ["loadstart", "progress", "load", "loadend"]) {
              xhr.addEventListener(eventType, event => events.push(
                `${eventType}:${event.loaded}:${event.total}:${event.lengthComputable}`
              ));
              xhr.upload.addEventListener(eventType, event => events.push(
                `upload.${eventType}:${event.loaded}:${event.total}:${event.lengthComputable}`
              ));
            }
            if (setBeforeOpen) xhr.responseType = type;
            const url = type === "json"
              ? "data:application/json,%7B%22value%22%3A7%7D"
              : "data:text/plain,ok";
            xhr.open("GET", url, false);
            if (!setBeforeOpen) xhr.responseType = type;
            xhr.send();
            let response;
            if (xhr.responseType === "arraybuffer") response = xhr.response.byteLength;
            else if (xhr.responseType === "blob") response = `${xhr.response.size}:${xhr.response.type}`;
            else if (xhr.responseType === "json") response = xhr.response.value;
            else response = xhr.responseText;
            return {
              type,
              setBeforeOpen,
              responseType: xhr.responseType,
              response,
              readyState: xhr.readyState,
              status: xhr.status,
              events,
            };
          };
          const matrix = [];
          for (const setBeforeOpen of [true, false]) {
            for (const type of ["arraybuffer", "blob", "json", "text", "document"]) {
              matrix.push(probe(type, setBeforeOpen));
            }
          }
          postMessage(matrix);
          close();
        })();
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );

    let observed = recv_post_json(&mut handle).await;
    let observed: serde_json::Value =
        serde_json::from_str(&observed).expect("worker responseType matrix should be JSON");
    let matrix = observed
        .as_array()
        .expect("worker responseType matrix should be an array");
    assert_eq!(matrix.len(), 10);
    for entry in matrix {
        let requested_type = entry["type"]
            .as_str()
            .expect("requested responseType should be a string");
        let effective_type = if requested_type == "document" {
            ""
        } else {
            requested_type
        };
        assert_eq!(entry["responseType"], effective_type);
        assert_eq!(entry["readyState"], 4);
        assert_eq!(entry["status"], 200);
        let loaded = if requested_type == "json" { 11 } else { 2 };
        assert_eq!(
            entry["events"],
            serde_json::json!([
                "readystatechange:1",
                "readystatechange:4",
                format!("load:{loaded}:{loaded}:true"),
                format!("loadend:{loaded}:{loaded}:true")
            ])
        );
        match requested_type {
            "arraybuffer" => assert_eq!(entry["response"], 2),
            "blob" => assert_eq!(entry["response"], "2:text/plain"),
            "json" => {
                assert_eq!(entry["response"], 7);
            }
            "text" | "document" => assert_eq!(entry["response"], "ok"),
            other => panic!("unexpected responseType matrix entry: {other}"),
        }
    }
}

#[tokio::test]
async fn worker_xhr_response_stage_continue_preserves_large_spooled_body() {
    ensure_v8();
    let body = "x".repeat(1024 * 1024 + 17);
    let expected_len = body.len();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/large-xhr.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        body,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    length: xhr.responseText.length,
                    first: xhr.responseText.slice(0, 1),
                    last: xhr.responseText.slice(-1),
                });
                close();
            };
            xhr.open("GET", "./large-xhr.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage large worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected large worker xhr pause, got {pending:?}");
    };
    let request = pending_worker_xhr_continue(pending.fetch_id, 67, &pending.info, true);
    handle.continue_pending_xhr(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage large worker xhr pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected large worker xhr response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 67);
    assert_eq!(
        info.response_body
            .read_chunk(expected_len - 1, 1)
            .expect("spooled body tail should be readable"),
        b"x"
    );

    handle.continue_pending_xhr_response(request, None, None);

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(r#"{{"status":200,"length":{expected_len},"first":"x","last":"x"}}"#)
    );
    server
        .await
        .expect("large worker xhr response-stage server should finish");
}

#[tokio::test]
async fn worker_xhr_auth_required_then_continue_with_auth_resolves() {
    ensure_v8();
    let (base_url, server) =
        spawn_basic_auth_http_server("/worker/xhr-auth.txt", "worker-xhr-area", "xhr-secret", 2)
            .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const readyStates = [];
            xhr.onreadystatechange = () => readyStates.push(xhr.readyState);
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    text: xhr.responseText,
                    readyStates,
                });
                close();
            };
            xhr.open("GET", "./xhr-auth.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker xhr auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_xhr_continue(pending.fetch_id, 41, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_xhr(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker xhr auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 41);
    assert_eq!(info.resource_type, SubresourceResourceType::Xhr);
    assert_eq!(info.challenge.source, "Server");
    assert_eq!(info.challenge.scheme, "basic");
    assert_eq!(info.challenge.realm, "worker-xhr-area");
    assert!(!info.intercept_response);
    assert_initial_worker_auth_network_headers(info.network_request_headers.as_deref());

    request.auth = Some(server_basic_auth_credentials());
    handle.continue_pending_xhr(request);

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr auth-success network record")
        .expect("worker channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert_initial_worker_auth_network_headers(record.network_request_headers());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Success { status, .. } if *status == 200
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":200,"text":"xhr-secret","readyStates":[1,2,3,4]}"#
    );
    server.await.expect("worker xhr auth server should finish");
}

#[tokio::test]
async fn worker_xhr_auth_required_then_fail_errors_without_exposing_challenge_body() {
    ensure_v8();
    let (base_url, server) =
        spawn_basic_auth_http_server("/worker/xhr-auth.txt", "worker-xhr-area", "xhr-secret", 1)
            .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.onerror = () => events.push("error");
            xhr.onload = () => events.push("load");
            xhr.onloadend = () => {
                events.push("loadend");
                postMessage({
                    status: xhr.status,
                    readyState: xhr.readyState,
                    text: xhr.responseText,
                    events,
                });
                close();
            };
            xhr.open("GET", "./xhr-auth.txt");
            xhr.send();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker xhr auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker xhr auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_xhr_continue(pending.fetch_id, 43, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_xhr(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker xhr auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker xhr auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 43);
    assert_eq!(info.challenge.realm, "worker-xhr-area");

    handle.fail_pending_xhr_auth(request, "worker xhr auth aborted".to_owned());

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":0,"readyState":4,"text":"","events":["error","loadend"]}"#
    );
    server
        .await
        .expect("worker xhr auth-fail server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_arraybuffer_preserves_response_bytes() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.bin",
        "HTTP/1.1 200 OK",
        "application/octet-stream",
        "A\0B".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            xhr.responseType = "arraybuffer";
            xhr.onloadend = () => {
                let responseTextState = "not-checked";
                try {
                    void xhr.responseText;
                    responseTextState = "readable";
                } catch (error) {
                    responseTextState = error && error.name;
                }
                postMessage({
                    status: xhr.status,
                    bytes: Array.from(new Uint8Array(xhr.response)).join(","),
                    responseTextState,
                });
                close();
            };
            xhr.open("GET", "./data.bin");
            xhr.send();
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"status":200,"bytes":"65,0,66","responseTextState":"InvalidStateError"}"#
    );
    server
        .await
        .expect("worker xhr arraybuffer server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_upload_dispatches_completion_events() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/upload",
        "HTTP/1.1 200 OK",
        "application/json",
        r#"{"ok":true}"#.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const payload = "upload=alpha&count=2";
            const uploadEvents = [];
            const uploadOrder = [];
            ["loadstart", "progress", "load", "loadend"].forEach((type) => {
                xhr.upload.addEventListener(type, (event) => {
                    uploadOrder.push(`listener-before:${event.type}`);
                });
                xhr.upload[`on${type}`] = (event) => {
                    uploadOrder.push(`handler:${event.type}`);
                };
                xhr.upload.addEventListener(type, (event) => {
                    uploadEvents.push([
                        event.type,
                        event.target === xhr.upload,
                        event.currentTarget === xhr.upload,
                        event.lengthComputable,
                        event.loaded,
                        event.total,
                    ].join(":"));
                    uploadOrder.push(`listener-after:${event.type}`);
                });
            });
            xhr.onloadend = () => {
                postMessage({
                    status: xhr.status,
                    response: xhr.responseText,
                    uploadEvents,
                    uploadOrder,
                });
                close();
            };
            xhr.open("POST", "./upload");
            xhr.setRequestHeader("Content-Type", "text/plain;charset=utf-8");
            xhr.send(payload);
        })();
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"status":200,"response":"{\"ok\":true}","uploadEvents":["loadstart:true:true:true:20:20","progress:true:true:true:20:20","load:true:true:true:20:20","loadend:true:true:true:20:20"],"uploadOrder":["listener-before:loadstart","handler:loadstart","listener-after:loadstart","listener-before:progress","handler:progress","listener-after:progress","listener-before:load","handler:load","listener-after:load","listener-before:loadend","handler:loadend","listener-after:loadend"]}"#
    );
    server
        .await
        .expect("worker xhr upload server should finish");
}

#[tokio::test]
async fn worker_fetch_uses_worker_script_base_url_and_resolves_response_text() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/data.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "hello from worker fetch".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const response = await fetch("./data.txt");
            postMessage({
                ok: response.ok,
                status: response.status,
                url: response.url,
                text: await response.text()
            });
            close();
        })().catch((error) => {
            postMessage({ error: String(error) });
            close();
        });
        "#
        .into(),
        script_url.clone(),
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        format!(
            r#"{{"ok":true,"status":200,"url":"{base_url}/assets/data.txt","text":"hello from worker fetch"}}"#
        )
    );
    server.await.expect("worker fetch server should finish");
}

#[tokio::test]
async fn worker_fetch_resolves_response_before_delayed_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed worker fetch server");
    let addr = listener.local_addr().expect("delayed worker fetch addr");
    let (release_body_tx, release_body_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept delayed worker fetch request");
        let _request = read_http_request_head(&mut stream)
            .await
            .expect("read delayed worker fetch request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write delayed worker fetch headers");
        let _ = release_body_rx.await;
        stream
            .write_all(b"hello world")
            .await
            .expect("write delayed worker fetch body");
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const response = await fetch("./delayed.txt");
            postMessage({ phase: "headers", status: response.status });
            postMessage({ phase: "body", text: await response.text() });
            close();
        })().catch((error) => {
            postMessage({ phase: "error", error: String(error), name: error && error.name });
            close();
        });
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
    );

    let headers = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for headers-first worker fetch")
        .expect("worker channel closed");
    assert_eq!(
        expect_post_json(headers),
        r#"{"phase":"headers","status":200}"#
    );
    release_body_tx
        .send(())
        .expect("delayed body receiver should still be waiting");
    let body = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for delayed worker fetch body")
        .expect("worker channel closed");
    assert_eq!(
        expect_post_json(body),
        r#"{"phase":"body","text":"hello world"}"#
    );
    server
        .await
        .expect("delayed worker fetch server should finish");
}

#[tokio::test]
async fn worker_fetch_streams_spooled_response_body_chunks() {
    ensure_v8();
    let body = "x".repeat(1024 * 1024 + 17);
    let expected_len = body.len();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/assets/large.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        body,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let script_url = format!("{base_url}/assets/main.js");
    let mut handle = spawn_worker_with_request_client(
        r#"
        (async () => {
            const response = await fetch("./large.txt");
            const reader = response.body.getReader();
            let chunks = 0;
            let total = 0;
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;
                chunks++;
                total += value.byteLength;
            }
            postMessage({ status: response.status, chunks, total });
            close();
        })().catch((error) => {
            postMessage({ error: String(error) });
            close();
        });
        "#
        .into(),
        script_url,
        loader,
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let payload = expect_post_json(msg);
    let payload = serde_json::from_str::<serde_json::Value>(&payload)
        .expect("worker fetch stream result should be JSON");
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["total"].as_u64(), Some(expected_len as u64));
    let chunks = payload["chunks"]
        .as_u64()
        .expect("worker fetch stream result should include chunk count");
    assert!(
        chunks > 1,
        "large response should be observed through multiple stream chunks: {payload}"
    );
    server.await.expect("worker fetch server should finish");
}

#[tokio::test]
async fn worker_fetch_applies_network_policy_extra_http_headers() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker header server");
    let addr = listener.local_addr().expect("worker header server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker fetch request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker fetch request");
        let received = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            received.len(),
            received
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker header response");
    });
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        (async () => {
            const response = await fetch("./api");
            postMessage(await response.text());
            close();
        })().catch((error) => {
            postMessage(String(error));
            close();
        });
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
        WorkerNetworkPolicy {
            extra_http_headers: vec![("x-cdp-test".to_owned(), "works-worker".to_owned())],
            ..WorkerNetworkPolicy::default()
        },
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""works-worker""#);
    server.await.expect("worker header server should finish");
}

#[tokio::test]
async fn worker_fetch_request_stage_interception_can_fulfill_synthetic_response() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("http://example.test/intercepted-worker-fetch");
                postMessage({
                    status: response.status,
                    header: response.headers.get("x-worker-intercept"),
                    text: await response.text(),
                });
            } catch (error) {
                postMessage({ error: String(error) });
            }
            close();
        };
        "#
        .into(),
        "http://example.test/worker/main.js".into(),
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader"),
        WorkerNetworkPolicy {
            network_partition_key: Some("credentialless-worker-fetch".to_owned()),
            ..WorkerNetworkPolicy::default()
        },
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch pause, got {pending:?}");
    };
    assert!(pending.info.network_request_handle.is_none());
    assert_eq!(pending.info.resource_type, SubresourceResourceType::Fetch);
    assert_eq!(
        pending.info.url.as_str(),
        "http://example.test/intercepted-worker-fetch"
    );
    assert_eq!(
        pending.network_partition_key.as_deref(),
        Some("credentialless-worker-fetch")
    );

    let request = pending_worker_fetch_continue(pending.fetch_id, 17, &pending.info, false);
    handle.fulfill_pending_fetch(
        request,
        202,
        vec![
            ("content-type".to_owned(), "text/plain".to_owned()),
            ("x-worker-intercept".to_owned(), "request-stage".to_owned()),
        ],
        RendererSyntheticResponseBody::from_bytes(b"fulfilled-worker-fetch".to_vec()),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":202,"header":"request-stage","text":"fulfilled-worker-fetch"}"#
    );
}

#[tokio::test]
async fn worker_subresource_request_handles_are_owner_unique() {
    ensure_v8();

    fn spawn_intercepted_fetch_worker() -> WorkerTestHandle {
        spawn_worker_with_request_client_and_network_policy(
            r#"
            onmessage = async () => {
                try {
                    await fetch("http://example.test/intercepted-worker-fetch");
                    postMessage("done");
                } catch (error) {
                    postMessage({ error: String(error) });
                }
                close();
            };
            "#
            .into(),
            "http://example.test/worker/main.js".into(),
            ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader"),
            WorkerNetworkPolicy {
                network_partition_key: Some("credentialless-worker-fetch".to_owned()),
                ..WorkerNetworkPolicy::default()
            },
        )
    }

    let mut first = spawn_intercepted_fetch_worker();
    let mut second = spawn_intercepted_fetch_worker();
    first.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    second.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    first.post_message(serialize_test_string("go"));
    second.post_message(serialize_test_string("go"));

    let first_pending = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for first worker fetch pause")
        .expect("first worker channel closed");
    let second_pending = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for second worker fetch pause")
        .expect("second worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(first_pending) = first_pending else {
        panic!("expected first worker fetch pause, got {first_pending:?}");
    };
    let WorkerToParentMessage::PendingSubresourceFetch(second_pending) = second_pending else {
        panic!("expected second worker fetch pause, got {second_pending:?}");
    };

    assert!(
        first_pending.info.network_request_handle.is_none(),
        "worker pause should not allocate a local request handle"
    );
    assert!(
        second_pending.info.network_request_handle.is_none(),
        "worker pause should not allocate a local request handle"
    );

    let first_request =
        pending_worker_fetch_continue(first_pending.fetch_id, 101, &first_pending.info, false);
    let second_request =
        pending_worker_fetch_continue(second_pending.fetch_id, 202, &second_pending.info, false);
    let first_handle = first_request
        .network_request_handle
        .expect("first owner-assigned worker fetch handle");
    let second_handle = second_request
        .network_request_handle
        .expect("second owner-assigned worker fetch handle");
    assert_ne!(
        first_handle, second_handle,
        "worker-owned request handles must carry owner identity"
    );

    first.fulfill_pending_fetch(
        first_request,
        200,
        vec![("content-type".to_owned(), "text/plain".to_owned())],
        RendererSyntheticResponseBody::from_bytes(b"first-worker-body".to_vec()),
    );
    second.fulfill_pending_fetch(
        second_request,
        200,
        vec![("content-type".to_owned(), "text/plain".to_owned())],
        RendererSyntheticResponseBody::from_bytes(b"second-worker-body".to_vec()),
    );

    let first_network = timeout(TIMEOUT, first.recv())
        .await
        .expect("timed out waiting for first worker network record")
        .expect("first worker channel closed");
    let first_record = expect_subresource_network_record(first_network);
    assert_eq!(first_record.request_handle(), Some(first_handle));

    let second_network = timeout(TIMEOUT, second.recv())
        .await
        .expect("timed out waiting for second worker network record")
        .expect("second worker channel closed");
    let second_record = expect_subresource_network_record(second_network);
    assert_eq!(second_record.request_handle(), Some(second_handle));

    assert_eq!(recv_post_json(&mut first).await, r#""done""#);
    assert_eq!(recv_post_json(&mut second).await, r#""done""#);

    first.terminate_and_join();
    second.terminate_and_join();
}

#[tokio::test]
async fn worker_fetch_continue_request_resolves_response_before_delayed_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind continued worker fetch delayed server");
    let addr = listener
        .local_addr()
        .expect("continued worker fetch delayed addr");
    let (release_body_tx, release_body_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept continued worker fetch request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read continued worker fetch request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("continued worker fetch request path");
        assert_eq!(path, "/worker/delayed.txt");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write continued worker fetch headers");
        let _ = release_body_rx.await;
        stream
            .write_all(b"hello world")
            .await
            .expect("write continued worker fetch body");
    });

    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./delayed.txt");
                postMessage({ phase: "headers", status: response.status });
                postMessage({ phase: "body", text: await response.text() });
            } catch (error) {
                postMessage({ phase: "error", error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for continued worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected continued worker fetch pause, got {pending:?}");
    };
    let request = pending_worker_fetch_continue(pending.fetch_id, 61, &pending.info, false);
    handle.continue_pending_fetch(request);

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"headers","status":200}"#
    );
    release_body_tx
        .send(())
        .expect("continued worker fetch body release should be received");
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"phase":"body","text":"hello world"}"#
    );
    server
        .await
        .expect("continued worker fetch delayed server should finish");
}

#[tokio::test]
async fn worker_fetch_response_stage_interception_pauses_before_resolving_response() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/api.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        "origin-worker-body".to_owned(),
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./api.txt");
                postMessage({
                    status: response.status,
                    header: response.headers.get("x-worker-response-stage"),
                    text: await response.text(),
                });
            } catch (error) {
                postMessage({ error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch pause, got {pending:?}");
    };
    let request = pending_worker_fetch_continue(pending.fetch_id, 23, &pending.info, true);
    handle.continue_pending_fetch(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected worker response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 23);
    assert_eq!(info.response_status, 200);
    assert_eq!(info.response_body.text().as_ref(), "origin-worker-body");

    handle.continue_pending_fetch_response(
        request,
        Some(203),
        Some(vec![
            ("content-type".to_owned(), "text/plain".to_owned()),
            ("x-worker-response-stage".to_owned(), "continued".to_owned()),
        ]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"status":203,"header":"continued","text":"origin-worker-body"}"#
    );
    server
        .await
        .expect("worker response-stage server should finish");
}

#[tokio::test]
async fn worker_fetch_response_stage_continue_preserves_large_spooled_body_stream() {
    ensure_v8();
    let body = "x".repeat(1024 * 1024 + 17);
    let expected_len = body.len();
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/large-fetch.txt",
        "HTTP/1.1 200 OK",
        "text/plain; charset=utf-8",
        body,
        Duration::ZERO,
    )])
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./large-fetch.txt");
                const reader = response.body.getReader();
                let chunks = 0;
                let total = 0;
                while (true) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    chunks++;
                    total += value.byteLength;
                }
                postMessage({ status: response.status, chunks, total });
            } catch (error) {
                postMessage({ error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage large worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected large worker fetch pause, got {pending:?}");
    };
    let request = pending_worker_fetch_continue(pending.fetch_id, 71, &pending.info, true);
    handle.continue_pending_fetch(request.clone());

    let response_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for response-stage large worker fetch pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::ResponsePaused(info),
    ) = response_pause
    else {
        panic!("expected large worker fetch response-stage pause, got {response_pause:?}");
    };
    assert_eq!(info.internal_id, 71);
    assert_eq!(
        info.response_body
            .read_chunk(expected_len - 1, 1)
            .expect("spooled body tail should be readable"),
        b"x"
    );

    handle.continue_pending_fetch_response(request, None, None);

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(r#"{{"status":200,"chunks":17,"total":{expected_len}}}"#)
    );
    server
        .await
        .expect("large worker fetch response-stage server should finish");
}

#[tokio::test]
async fn worker_fetch_auth_required_then_continue_with_auth_resolves() {
    ensure_v8();
    let (base_url, server) = spawn_basic_auth_http_server(
        "/worker/fetch-auth.txt",
        "worker-fetch-area",
        "fetch-secret",
        2,
    )
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./fetch-auth.txt");
                postMessage({ ok: true, status: response.status, text: await response.text() });
            } catch (error) {
                postMessage({ ok: false, error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker fetch auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_fetch_continue(pending.fetch_id, 47, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_fetch(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker fetch auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 47);
    assert_eq!(info.resource_type, SubresourceResourceType::Fetch);
    assert_eq!(info.challenge.source, "Server");
    assert_eq!(info.challenge.scheme, "basic");
    assert_eq!(info.challenge.realm, "worker-fetch-area");
    assert!(!info.intercept_response);
    assert_initial_worker_auth_network_headers(info.network_request_headers.as_deref());

    let expected_request_handle = Some(owner_assigned_request_handle(47));
    request.auth = Some(server_basic_auth_credentials());
    handle.continue_pending_fetch(request);

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth-success network record")
        .expect("worker channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.request_handle(), expected_request_handle);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert_initial_worker_auth_network_headers(record.network_request_headers());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Success { status, .. } if *status == 200
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"ok":true,"status":200,"text":"fetch-secret"}"#
    );
    server
        .await
        .expect("worker fetch auth server should finish");
}

#[tokio::test]
async fn worker_fetch_auth_required_then_fail_rejects_without_exposing_challenge_body() {
    ensure_v8();
    let (base_url, server) = spawn_basic_auth_http_server(
        "/worker/fetch-auth.txt",
        "worker-fetch-area",
        "fetch-secret",
        1,
    )
    .await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                const response = await fetch("./fetch-auth.txt");
                postMessage({ ok: true, status: response.status, text: await response.text() });
            } catch (error) {
                postMessage({ ok: false, error: String(error) });
            }
            close();
        };
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let pending = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for request-stage worker fetch auth pause")
        .expect("worker channel closed");
    let WorkerToParentMessage::PendingSubresourceFetch(pending) = pending else {
        panic!("expected worker fetch auth request pause, got {pending:?}");
    };
    let mut request = pending_worker_fetch_continue(pending.fetch_id, 53, &pending.info, false);
    request.handle_auth_requests = true;
    handle.continue_pending_fetch(request.clone());

    let auth_pause = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth challenge")
        .expect("worker channel closed");
    let WorkerToParentMessage::SubresourceContinue(PendingSubresourceContinueEvent::AuthRequired(
        info,
    )) = auth_pause
    else {
        panic!("expected worker fetch auth challenge, got {auth_pause:?}");
    };
    assert_eq!(info.internal_id, 53);
    assert_eq!(info.challenge.realm, "worker-fetch-area");

    let expected_request_handle = Some(owner_assigned_request_handle(53));
    handle.fail_pending_fetch_auth(request, "worker fetch auth aborted".to_owned());

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out waiting for worker fetch auth-fail network record")
        .expect("worker channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.request_handle(), expected_request_handle);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "worker fetch auth aborted"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"ok":false,"error":"TypeError: worker fetch auth aborted"}"#
    );
    server
        .await
        .expect("worker fetch auth-fail server should finish");
}

#[tokio::test]
async fn worker_importscripts_applies_loader_network_policy_extra_http_headers() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker importScripts header server");
    let addr = listener
        .local_addr()
        .expect("worker importScripts header server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker importScripts request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker importScripts request");
        let received = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let body = format!("postMessage({received:?}); close();");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker importScripts response");
    });
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        importScripts("./imported.js");
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
        WorkerNetworkPolicy {
            extra_http_headers: vec![(
                "x-cdp-test".to_owned(),
                "works-worker-importscripts".to_owned(),
            )],
            ..WorkerNetworkPolicy::default()
        },
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""works-worker-importscripts""#);
    server
        .await
        .expect("worker importScripts header server should finish");
}

#[tokio::test]
async fn module_worker_dependency_applies_loader_network_policy_extra_http_headers() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind module worker dependency header server");
    let addr = listener
        .local_addr()
        .expect("module worker dependency header server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept module worker dependency request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read module worker dependency request");
        let received = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("x-cdp-test")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        let body = format!("export default {received:?};");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write module worker dependency response");
    });
    let loader = ResourceRequestClient::new(&FetchConfig::default())
        .expect("module worker dependency loader");
    let mut handle = spawn_worker_with_request_client_and_kind_and_network_policy(
        r#"
        import headerValue from "./dep.js";
        postMessage(headerValue);
        close();
        "#
        .into(),
        format!("http://{addr}/worker/main.js"),
        loader,
        WorkerScriptKind::Module,
        WorkerNetworkPolicy {
            extra_http_headers: vec![(
                "x-cdp-test".to_owned(),
                "works-module-worker-dependency".to_owned(),
            )],
            ..WorkerNetworkPolicy::default()
        },
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""works-module-worker-dependency""#);
    server
        .await
        .expect("module worker dependency header server should finish");
}

#[tokio::test]
async fn worker_fetch_missing_input_rejects_with_type_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            try {
                await fetch();
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    isTypeError: error instanceof TypeError
                });
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","isTypeError":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_bad_port_rejects_before_transport() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (async () => {
            try {
                await fetch("http://example.test:25/blocked-port");
                postMessage("unexpected");
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    hasBadPortMessage: String(error && error.message).includes("blocked bad port")
                });
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(record.url().as_str(), "http://example.test:25/blocked-port");
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("blocked bad port")
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","hasBadPortMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_file_url_rejects_before_interception_or_transport() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                await fetch("file:///moli-policy-must-not-open");
                postMessage({ fulfilled: true });
            } catch (error) {
                postMessage({
                    name: error && error.name,
                    message: error && error.message,
                    isTypeError: error instanceof TypeError,
                });
            }
            close();
        };
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        match timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker file fetch rejection")
            .expect("worker channel closed")
        {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unsupported worker fetch must not reach interception: {other:?}"),
        }
    }

    let record = network.expect("worker file fetch should record a network failure");
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert_eq!(
        record.outcome(),
        &SubresourceNetworkOutcome::Failure {
            error_text: "URL scheme \"file\" is not supported.".to_owned(),
        }
    );
    assert_eq!(
        post.expect("worker file fetch should expose a TypeError"),
        r#"{"name":"TypeError","message":"URL scheme \"file\" is not supported.","isTypeError":true}"#
    );
}

#[tokio::test]
async fn worker_network_offline_fetch_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        (async () => {
            try {
                await fetch("http://example.test/offline/worker-fetch");
                postMessage("unexpected");
            } catch (error) {
                postMessage(String(error));
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        WorkerNetworkPolicy {
            network_offline: true,
            ..WorkerNetworkPolicy::default()
        },
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/offline/worker-fetch"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "Network emulation offline"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""TypeError: Network emulation offline""#
    );
}

#[tokio::test]
async fn worker_fetch_blocked_url_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        (async () => {
            try {
                await fetch("http://example.test/blocked/worker-fetch");
                postMessage("unexpected");
            } catch (error) {
                postMessage(String(error));
            }
            close();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        vec!["http://example.test/blocked/*".to_owned()],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/blocked/worker-fetch"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "net::ERR_BLOCKED_BY_CLIENT"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""TypeError: net::ERR_BLOCKED_BY_CLIENT""#
    );
}

#[tokio::test]
async fn worker_fetch_connection_refused_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let (base_url, server) =
        spawn_connection_drop_http_server("/worker-fetch-connection-refused").await;
    let url = format!("{base_url}/worker-fetch-connection-refused");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasMessage: String(error && error.message).length > 0,
                    stringStartsWithTypeError: String(error).startsWith("TypeError"),
                }});
            }}
            close();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for worker fetch DNS failure output (post={}, network={})",
                    post.is_some(),
                    network.is_some()
                )
            })
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("worker fetch connection failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
    ));
    assert_eq!(
        post.expect("worker fetch connection failure should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasMessage":true,"stringStartsWithTypeError":true}"#
    );
    server
        .await
        .expect("worker fetch connection-drop server should finish");
}

#[tokio::test]
async fn worker_fetch_dns_failure_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let url = "http://moli-dns-failure.invalid./worker-fetch-dns-failure";
    let url_literal = serde_json::to_string(url).expect("serialize worker fetch url");
    let loader =
        ResourceRequestClient::new(&dns_failure_fetch_config()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasMessage: String(error && error.message).length > 0,
                    stringStartsWithTypeError: String(error).startsWith("TypeError"),
                }});
            }}
            close();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let record = network.expect("worker fetch DNS failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
        panic!(
            "expected worker fetch DNS failure, got {:?}",
            record.outcome()
        );
    };
    assert!(
        error_text.to_ascii_lowercase().contains("resolv"),
        "expected DNS-resolution error text, got {error_text:?}"
    );
    assert_eq!(
        post.expect("worker fetch DNS failure should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasMessage":true,"stringStartsWithTypeError":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_redirect_error_rejects_before_following_redirect() {
    ensure_v8();
    let (base_url, server) =
        spawn_single_redirect_http_server("/worker-fetch-redirect-error", "/target").await;
    let url = format!("{base_url}/worker-fetch-redirect-error");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal}, {{ redirect: "error" }});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasRedirectModeMessage: String(error && error.message).includes("redirect mode error"),
                }});
            }}
            close();
        }})();
        "#
        ),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    server
        .await
        .expect("worker fetch redirect-error server should finish");
    let record = network.expect("worker fetch redirect error should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("redirect mode error")
    ));
    assert_eq!(
        post.expect("worker fetch redirect error should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasRedirectModeMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_manual_redirect_returns_opaqueredirect_filtered_response() {
    ensure_v8();
    let (base_url, server) =
        spawn_single_redirect_http_server("/worker-fetch-redirect-manual", "/target").await;
    let url = format!("{base_url}/worker-fetch-redirect-manual");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            const response = await fetch({url_literal}, {{ redirect: "manual" }});
            const clone = response.clone();
            const bodyUsedBefore = response.bodyUsed;
            const text = await response.text();
            const cloneText = await clone.text();
            postMessage({{
                type: response.type,
                status: response.status,
                ok: response.ok,
                statusText: response.statusText,
                redirected: response.redirected,
                urlIsEmpty: response.url === "",
                bodyIsNull: response.body === null,
                headers: Array.from(response.headers),
                bodyUsedBefore,
                bodyUsedAfter: response.bodyUsed,
                text,
                cloneType: clone.type,
                cloneStatus: clone.status,
                cloneBodyIsNull: clone.body === null,
                cloneText,
            }});
            close();
        }})().catch((error) => {{
            postMessage({{ error: String(error), name: error && error.name }});
            close();
        }});
        "#
        ),
        format!("{base_url}/worker/main.js"),
        loader,
    );

    let post = recv_post_json(&mut handle).await;
    server
        .await
        .expect("worker fetch manual-redirect server should finish");
    assert_eq!(
        post,
        r#"{"type":"opaqueredirect","status":0,"ok":false,"statusText":"","redirected":false,"urlIsEmpty":true,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":true,"text":"","cloneType":"opaqueredirect","cloneStatus":0,"cloneBodyIsNull":true,"cloneText":""}"#
    );
}

#[tokio::test]
async fn worker_fetch_no_cors_cross_origin_returns_opaque_filtered_response() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors fetch server");
    let addr = listener.local_addr().expect("worker no-cors fetch addr");
    let fetch_url = format!("http://{addr}/worker-no-cors-data");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors fetch url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors fetch request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors fetch request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsecret worker",
            )
            .await
            .expect("write worker no-cors fetch response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
            const clone = response.clone();
            const bodyUsedBefore = response.bodyUsed;
            const text = await response.text();
            const cloneText = await clone.text();
            postMessage({{
                type: response.type,
                status: response.status,
                ok: response.ok,
                statusText: response.statusText,
                url: response.url,
                redirected: response.redirected,
                bodyIsNull: response.body === null,
                headers: Array.from(response.headers),
                bodyUsedBefore,
                bodyUsedAfter: response.bodyUsed,
                text,
                cloneType: clone.type,
                cloneStatus: clone.status,
                cloneBodyIsNull: clone.body === null,
                cloneText,
            }});
            close();
        }})().catch((error) => {{
            postMessage({{ error: String(error), name: error && error.name }});
            close();
        }});
        "#
        ),
        worker_script_url,
        loader,
    );

    let post = recv_post_json(&mut handle).await;
    let request = request_rx
        .await
        .expect("worker no-cors server should capture request");
    server
        .await
        .expect("worker no-cors fetch server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post,
        r#"{"type":"opaque","status":0,"ok":false,"statusText":"","url":"","redirected":false,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":true,"text":"","cloneType":"opaque","cloneStatus":0,"cloneBodyIsNull":true,"cloneText":""}"#
    );
}

#[tokio::test]
async fn worker_fetch_no_cors_opaque_response_blocking_returns_empty_opaque_response() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors ORB server");
    let addr = listener.local_addr().expect("worker no-cors ORB addr");
    let fetch_url = format!("http://{addr}/worker-orb-data");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors ORB url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors ORB request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors ORB request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"secret\":true}",
            )
            .await
            .expect("write worker no-cors ORB response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{
                    fulfilled: true,
                    type: response.type,
                    status: response.status,
                    url: response.url,
                    bodyIsNull: response.body === null,
                    headerCount: Array.from(response.headers).length,
                }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasOrbMessage: String(error && error.message).includes("OpaqueResponseBlocking"),
                }});
            }}
            close();
        }})();
        "#
        ),
        worker_script_url,
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let request = request_rx
        .await
        .expect("worker no-cors ORB server should capture request");
    server
        .await
        .expect("worker no-cors ORB server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post.expect("worker no-cors ORB should post response"),
        r#"{"fulfilled":true,"type":"opaque","status":0,"url":"","bodyIsNull":true,"headerCount":0}"#
    );
    let record = network.expect("worker no-cors ORB should record network failure");
    assert_eq!(record.url().as_str(), fetch_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == crate::network_host::ABORTED_ERROR_TEXT
    ));
}

#[tokio::test]
async fn worker_fetch_no_cors_image_rejects_when_worker_policy_requires_coep_corp() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors COEP server");
    let addr = listener.local_addr().expect("worker no-cors COEP addr");
    let fetch_url = format!("http://{addr}/worker-coep-image");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors COEP url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors COEP request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors COEP request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsecret worker",
            )
            .await
            .expect("write worker no-cors COEP response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        (async () => {{
            try {{
                await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCoepMessage: String(error && error.message).includes("Cross-Origin-Embedder-Policy"),
                }});
            }}
            close();
        }})();
        "#
            ),
            worker_script_url,
        )
        .with_request_client(loader)
        .with_policy_context(crate::types::SubresourcePolicyContext {
            cross_origin_embedder_policy:
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp,
            ..Default::default()
        }),
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let request = request_rx
        .await
        .expect("worker no-cors COEP server should capture request");
    server
        .await
        .expect("worker no-cors COEP server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post.expect("worker no-cors COEP should post rejection"),
        r#"{"name":"TypeError","isTypeError":true,"hasCoepMessage":true}"#
    );
    let record = network.expect("worker no-cors COEP should record network failure");
    assert_eq!(record.url().as_str(), fetch_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Cross-Origin-Embedder-Policy")
    ));
}

#[tokio::test]
async fn worker_fetch_no_cors_orb_allows_mislabeled_png_body() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors ORB image server");
    let addr = listener
        .local_addr()
        .expect("worker no-cors ORB image addr");
    let fetch_url = format!("http://{addr}/worker-image-as-html");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors ORB image url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors ORB image request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors ORB image request");
        let _ = request_tx.send(request);
        let body = b"\x89PNG\r\n\x1A\nworker";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker no-cors ORB image headers");
        stream
            .write_all(body)
            .await
            .expect("write worker no-cors ORB image body");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{
                    type: response.type,
                    status: response.status,
                    bodyIsNull: response.body === null,
                }});
            }} catch (error) {{
                postMessage({{ rejected: true, message: String(error && error.message) }});
            }}
            close();
        }})();
        "#
        ),
        worker_script_url,
        loader,
    );

    let post = recv_post_json(&mut handle).await;
    let request = request_rx
        .await
        .expect("worker no-cors ORB image server should capture request");
    server
        .await
        .expect("worker no-cors ORB image server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(post, r#"{"type":"opaque","status":0,"bodyIsNull":true}"#);
}

#[tokio::test]
async fn worker_fetch_no_cors_cross_origin_resource_policy_blocks_response() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker no-cors CORP server");
    let addr = listener.local_addr().expect("worker no-cors CORP addr");
    let fetch_url = format!("http://{addr}/worker-corp-data");
    let fetch_url_literal =
        serde_json::to_string(&fetch_url).expect("serialize worker no-cors CORP url");
    let (request_tx, request_rx) = oneshot::channel::<String>();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker no-cors CORP request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker no-cors CORP request");
        let _ = request_tx.send(request);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nCross-Origin-Resource-Policy: same-origin\r\nContent-Length: 13\r\nConnection: close\r\n\r\nsecret worker",
            )
            .await
            .expect("write worker no-cors CORP response");
    });
    let worker_script_url = unused_local_http_url("/worker/main.js").await;
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCorpMessage: String(error && error.message).includes("Cross-Origin-Resource-Policy"),
                }});
            }}
            close();
        }})();
        "#
        ),
        worker_script_url,
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    let request = request_rx
        .await
        .expect("worker no-cors CORP server should capture request");
    server
        .await
        .expect("worker no-cors CORP server should finish");
    assert!(request.contains("Sec-Fetch-Mode: no-cors\r\n"));
    assert_eq!(
        post.expect("worker no-cors CORP should post rejection"),
        r#"{"name":"TypeError","isTypeError":true,"hasCorpMessage":true}"#
    );
    let record = network.expect("worker no-cors CORP should record network failure");
    assert_eq!(record.url().as_str(), fetch_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Cross-Origin-Resource-Policy")
    ));
}

#[tokio::test]
async fn worker_fetch_redirect_loop_rejects_and_reports_subresource_failure() {
    ensure_v8();
    let (base_url, server) = spawn_redirect_loop_http_server("/worker-fetch-loop").await;
    let url = format!("{base_url}/worker-fetch-loop");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasRedirectLimitMessage: String(error && error.message).includes("redirect limit exceeded"),
                }});
            }}
            close();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    server
        .await
        .expect("worker fetch redirect-loop server should finish");
    let record = network.expect("worker fetch redirect loop should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("redirect limit exceeded")
    ));
    assert_eq!(
        post.expect("worker fetch redirect loop should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasRedirectLimitMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_cross_origin_redirect_without_cors_rejects_and_reports_failure() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_without_cors_http_servers(
            "/worker-fetch-cors-redirect-deny",
            "/worker-fetch-cors-denied-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-fetch-cors-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (async () => {{
            try {{
                await fetch({url_literal});
                postMessage({{ fulfilled: true }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCorsMessage: String(error && error.message).includes("CORS check failed"),
                }});
            }}
            close();
        }})();
        "#
        ),
        format!("{source_base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker CORS redirect source server should finish");
    target_server
        .await
        .expect("worker CORS redirect target server should finish");
    let record = network.expect("worker fetch CORS redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
        panic!("expected CORS network failure, got {:?}", record.outcome());
    };
    assert_eq!(error_text, crate::network_host::FAILED_ERROR_TEXT);
    assert_eq!(
        post.expect("worker fetch CORS redirect should post rejection surface"),
        r#"{"name":"TypeError","isTypeError":true,"hasCorsMessage":true}"#
    );
}

#[tokio::test]
async fn worker_fetch_cross_origin_redirect_final_url_obeys_connect_src() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_with_cors_http_servers(
            "/worker-fetch-csp-redirect-deny",
            "/worker-fetch-csp-target",
            "worker-csp-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-fetch-csp-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker fetch url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker fetch loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        (async () => {{
            const events = [];
            addEventListener("securitypolicyviolation", event => {{
                events.push({{
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition,
                }});
            }});
            try {{
                const response = await fetch({url_literal});
                postMessage({{ fulfilled: true, text: await response.text() }});
            }} catch (error) {{
                postMessage({{
                    name: error && error.name,
                    isTypeError: error instanceof TypeError,
                    hasCspMessage: String(error && error.message).includes("Content Security Policy"),
                    events,
                }});
            }}
            close();
        }})();
        "#
            ),
            format!("{source_base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_content_security_policies(vec!["connect-src 'self'".to_owned()]),
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker CSP redirect source server should finish");
    target_server
        .await
        .expect("worker CSP redirect target server should finish");
    let record = network.expect("worker fetch CSP redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Content Security Policy")
    ));
    assert_eq!(
        post.expect("worker fetch CSP redirect should post rejection surface"),
        format!(
            r#"{{"name":"TypeError","isTypeError":true,"hasCspMessage":true,"events":[{{"blockedURI":"{url}","effectiveDirective":"connect-src","disposition":"enforce"}}]}}"#
        )
    );
}

#[tokio::test]
async fn worker_blocked_url_pattern_update_reaches_running_worker() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = async () => {
            try {
                await fetch("http://example.test/blocked/live-worker-fetch");
                postMessage("unexpected");
            } catch (error) {
                postMessage(String(error));
            }
            close();
        };
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );

    handle.set_blocked_url_patterns(&["http://example.test/blocked/*".to_owned()]);
    handle.post_message(serialize_test_string("go"));

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/blocked/live-worker-fetch"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "net::ERR_BLOCKED_BY_CLIENT"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""TypeError: net::ERR_BLOCKED_BY_CLIENT""#
    );
}

#[tokio::test]
async fn worker_classic_websocket_echo_round_trips_on_worker_loop() {
    ensure_v8();
    let (url, server) = spawn_text_echo_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
            const events = [];
            const socket = new WebSocket({url:?});
            events.push(`construct:${{socket.readyState}}:${{socket instanceof WebSocket}}`);
            socket.onopen = () => {{
                events.push(`open:${{socket.readyState}}`);
                socket.send("hello-worker-ws");
            }};
            socket.onmessage = (event) => {{
                events.push(`message:${{event.data}}:${{event instanceof MessageEvent}}`);
                socket.close(1000, "done");
            }};
            socket.onclose = (event) => {{
                events.push(`close:${{event.code}}:${{event.wasClean}}:${{event instanceof CloseEvent}}`);
                postMessage(events.join("|"));
                close();
            }};
            socket.onerror = () => {{
                events.push("error");
            }};
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""construct:0:true|open:1|message:hello-worker-ws:true|close:1000:true:true""#
    );
    server.await.expect("worker websocket server should finish");
}

#[tokio::test]
async fn worker_classic_websocket_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (url, server) = spawn_text_echo_websocket_server().await;
    let script_url = "http://127.0.0.1/worker/main.js";
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
            const cspEvents = [];
            addEventListener("securitypolicyviolation", event => {{
                cspEvents.push({{
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                }});
            }});
            const socket = new WebSocket({url:?});
            socket.onopen = () => socket.send("report-only-ws");
            socket.onmessage = (event) => {{
                postMessage({{ events: cspEvents, data: event.data }});
                socket.close(1000, "done");
                close();
            }};
            socket.onerror = () => {{
                postMessage({{ events: cspEvents, error: "ws-error" }});
                close();
            }};
            "#
            ),
            script_url.to_owned(),
        )
        .with_request_client(loader)
        .with_content_security_report_only_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"{url}","documentURI":"{script_url}","originalPolicy":"connect-src 'none'","disposition":"report","instance":true}}],"data":"report-only-ws"}}"#
        )
    );
    server
        .await
        .expect("worker websocket report-only server should finish");
}

#[tokio::test]
async fn worker_websocket_csp_block_precedes_mixed_content_rejection() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition
                });
            });
            let outcome;
            try {
                const socket = new WebSocket("ws:/common/blank.html");
                outcome = `socket:${socket.readyState}:${socket.url}`;
            } catch (error) {
                outcome = `throw:${error.name}`;
            }
            postMessage({ outcome, events });
            close();
            "#
            .to_owned(),
            "http://localhost:8000/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"outcome":"socket:0:ws://common/blank.html","events":[{"blockedURI":"ws://common/blank.html","effectiveDirective":"connect-src","disposition":"enforce"}]}"#
    );
}

#[tokio::test]
async fn worker_classic_websocket_connect_src_self_allows_same_host_ws() {
    ensure_v8();
    let (url, server) = spawn_text_echo_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let ws_url = url::Url::parse(&url).expect("websocket url should parse");
    let worker_script_url = format!(
        "http://{}:{}/worker/main.js",
        ws_url.host_str().expect("websocket host"),
        ws_url.port_or_known_default().expect("websocket port")
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
            const socket = new WebSocket({url:?});
            socket.onopen = () => {{
                socket.send("self-csp");
            }};
            socket.onmessage = (event) => {{
                postMessage(event.data);
                socket.close(1000, "done");
                close();
            }};
            socket.onerror = () => {{
                postMessage("error");
                close();
            }};
            "#
            ),
            worker_script_url,
        )
        .with_request_client(loader)
        .with_content_security_policies(vec!["connect-src 'self'".to_owned()]),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""self-csp""#);
    server
        .await
        .expect("worker websocket self CSP server should finish");
}

#[tokio::test]
async fn worker_classic_websocket_applies_network_policy_extra_http_headers() {
    ensure_v8();
    let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        format!(
            r#"
            const socket = new WebSocket({url:?});
            socket.onopen = () => {{
                postMessage("opened");
                socket.close();
                close();
            }};
            socket.onerror = () => {{
                postMessage("error");
                close();
            }};
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
        WorkerNetworkPolicy {
            extra_http_headers: vec![("x-cdp-test".to_owned(), "worker-ws".to_owned())],
            ..WorkerNetworkPolicy::default()
        },
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""opened""#);
    let headers = timeout(TIMEOUT, headers_rx)
        .await
        .expect("timed out waiting for websocket headers")
        .expect("websocket header sender dropped");
    let received = headers
        .iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("x-cdp-test")
                .then_some(value.as_str())
        })
        .unwrap_or_default();
    assert_eq!(received, "worker-ws");
    server
        .await
        .expect("worker websocket header server should finish");
}

#[tokio::test]
async fn worker_classic_websocket_handshake_set_cookie_updates_cookie_store() {
    ensure_v8();
    let (url, server) = spawn_set_cookie_websocket_server().await;
    let ws_url = url::Url::parse(&url).expect("websocket url");
    let cookie_url = moli_websocket::websocket_cookie_url(&ws_url);
    let cookie_store = moli_cookie_jar::new_shared_browser_cookie_store();
    let loader =
        ResourceRequestClient::new_with_cookie_store(&FetchConfig::default(), cookie_store.clone())
            .expect("worker websocket cookie loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
            const socket = new WebSocket({url:?});
            socket.onopen = () => {{
                postMessage("opened");
                socket.close();
                close();
            }};
            socket.onerror = () => {{
                postMessage("error");
                close();
            }};
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""opened""#);
    server
        .await
        .expect("worker websocket set-cookie server should finish");

    let cookie_header = moli_fetch::cookie_header_for_request(
        &cookie_store,
        &cookie_url,
        moli_cookie_jar::NetworkCookieRequestContext::subresource("GET"),
    )
    .expect("cookie header lookup should succeed");
    assert_eq!(cookie_header.as_deref(), Some("ws_response_cookie=ok"));
}

#[tokio::test]
async fn worker_classic_websocket_blocked_url_reports_network_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        const events = [];
        const socket = new WebSocket("ws://127.0.0.1/blocked/worker-ws");
        socket.onerror = () => {
            events.push(`error:${socket.readyState}`);
        };
        socket.onclose = (event) => {
            events.push(`close:${event.code}:${event.wasClean}:${socket.readyState}`);
            postMessage(events.join("|"));
            close();
        };
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        vec!["ws://127.0.0.1/blocked/*".to_owned()],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.url().as_str(), "ws://127.0.0.1/blocked/worker-ws");
    assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
    assert!(record.websocket_socket_id().is_some());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "net::ERR_BLOCKED_BY_CLIENT"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""error:3|close:1006:false:3""#
    );
}

#[tokio::test]
async fn worker_classic_websocket_offline_reports_network_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        const events = [];
        const socket = new WebSocket("ws://127.0.0.1/offline/worker-ws");
        socket.onerror = () => {
            events.push(`error:${socket.readyState}`);
        };
        socket.onclose = (event) => {
            events.push(`close:${event.code}:${event.wasClean}:${socket.readyState}`);
            postMessage(events.join("|"));
            close();
        };
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        WorkerNetworkPolicy {
            network_offline: true,
            ..WorkerNetworkPolicy::default()
        },
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.url().as_str(), "ws://127.0.0.1/offline/worker-ws");
    assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
    assert!(record.websocket_socket_id().is_some());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "Network emulation offline"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""error:3|close:1006:false:3""#
    );
}

#[tokio::test]
async fn worker_importscripts_websocket_resolves_against_imported_script_url() {
    ensure_v8();
    let imported_script = r#"
        const events = [];
        const socket = new WebSocket("./blocked/imported-ws");
        socket.onerror = () => {
            events.push(`error:${socket.readyState}`);
        };
        socket.onclose = (event) => {
            events.push(`close:${event.code}:${event.wasClean}:${socket.readyState}`);
            postMessage({ url: socket.url, events: events.join("|") });
            close();
        };
    "#;
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/imported/ws.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        imported_script.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let websocket_base_url = base_url.replacen("http://", "ws://", 1);
    let expected_url = format!("{websocket_base_url}/worker/imported/blocked/imported-ws");
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        importScripts("./imported/ws.js");
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        vec![format!("{websocket_base_url}/worker/imported/blocked/*")],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.url().as_str(), expected_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
    assert!(record.websocket_socket_id().is_some());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "net::ERR_BLOCKED_BY_CLIENT"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(r#"{{"url":"{expected_url}","events":"error:3|close:1006:false:3"}}"#)
    );
    server
        .await
        .expect("importScripts websocket server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_bad_port_errors_before_transport() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.addEventListener('loadend', () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                });
                close();
            });
            xhr.open('GET', 'http://example.test:25/blocked-port');
            xhr.send();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(record.url().as_str(), "http://example.test:25/blocked-port");
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("blocked bad port")
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_blocked_url_reports_error_after_loadstart() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.addEventListener('loadend', () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                });
                close();
            });
            xhr.open('GET', 'http://example.test/blocked/worker-xhr');
            xhr.send();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        vec!["http://example.test/blocked/*".to_owned()],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/blocked/worker-xhr"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "net::ERR_BLOCKED_BY_CLIENT"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_offline_reports_error_after_loadstart() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        (() => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.addEventListener('loadend', () => {
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                });
                close();
            });
            xhr.open('GET', 'http://example.test/offline/worker-xhr');
            xhr.send();
        })();
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        WorkerNetworkPolicy {
            network_offline: true,
            ..WorkerNetworkPolicy::default()
        },
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match network {
        WorkerToParentMessage::SubresourceNetwork(record) => {
            assert_eq!(
                record.url().as_str(),
                "http://example.test/offline/worker-xhr"
            );
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == "Network emulation offline"
            ));
        }
        other => panic!("expected worker subresource network record, got {other:?}"),
    }

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_connection_refused_reports_error_after_loadstart() {
    ensure_v8();
    let (base_url, server) =
        spawn_connection_drop_http_server("/worker-xhr-connection-refused").await;
    let url = format!("{base_url}/worker-xhr-connection-refused");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }
    let record = network.expect("worker XHR connection failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
    ));
    assert_eq!(
        post.expect("worker XHR connection failure should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
    server
        .await
        .expect("worker XHR connection-drop server should finish");
}

#[tokio::test]
async fn worker_xmlhttprequest_file_url_rejects_before_interception_or_transport() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {
                events.push('loadend');
                postMessage({
                    readyState: xhr.readyState,
                    status: xhr.status,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    events,
                });
                close();
            });
            xhr.open('GET', 'file:///moli-policy-must-not-open');
            xhr.send();
        };
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        match timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for worker file XHR rejection")
            .expect("worker channel closed")
        {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unsupported worker XHR must not reach interception: {other:?}"),
        }
    }

    let record = network.expect("worker file XHR should record a network failure");
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert_eq!(
        record.outcome(),
        &SubresourceNetworkOutcome::Failure {
            error_text: "URL scheme \"file\" is not supported.".to_owned(),
        }
    );
    assert_eq!(
        post.expect("worker file XHR should expose a network error surface"),
        r#"{"readyState":4,"status":0,"responseURL":"","responseText":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn synchronous_worker_xhr_file_url_throws_network_error_without_progress_events() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client(
        r#"
        onmessage = () => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('loadend', () => events.push('loadend'));
            xhr.open('GET', 'file:///moli-policy-must-not-open', false);
            let error = null;
            try {
                xhr.send();
            } catch (caught) {
                error = {
                    name: caught && caught.name,
                    message: caught && caught.message,
                    isDomException: caught instanceof DOMException,
                };
            }
            postMessage({
                error,
                events,
                readyState: xhr.readyState,
                status: xhr.status,
            });
            close();
        };
        "#
        .into(),
        "https://example.test/worker/main.js".into(),
        worker_test_request_client(),
    );
    handle.set_fetch_subresource_interception(true, Some(SubresourceResourceType::Xhr));
    handle.post_message(serialize_test_string("go"));

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        match timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for synchronous worker file XHR rejection")
            .expect("worker channel closed")
        {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => post = Some(stringify_payload(&payload)),
            other => panic!("unsupported synchronous worker XHR reached interception: {other:?}"),
        }
    }

    let record = network.expect("synchronous worker file XHR should record a network failure");
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert_eq!(
        record.outcome(),
        &SubresourceNetworkOutcome::Failure {
            error_text: "URL scheme \"file\" is not supported.".to_owned(),
        }
    );
    assert_eq!(
        post.expect("synchronous worker file XHR should throw NetworkError"),
        r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'file:///moli-policy-must-not-open'.","isDomException":true},"events":["readystatechange:1"],"readyState":4,"status":0}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_dns_failure_reports_error_after_loadstart() {
    ensure_v8();
    let url = "http://moli-dns-failure.invalid./worker-xhr-dns-failure";
    let url_literal = serde_json::to_string(url).expect("serialize worker xhr url");
    let loader =
        ResourceRequestClient::new(&dns_failure_fetch_config()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for worker XHR DNS failure output (post={}, network={})",
                    post.is_some(),
                    network.is_some()
                )
            })
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }
    let record = network.expect("worker XHR DNS failure should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
        panic!(
            "expected worker XHR DNS failure, got {:?}",
            record.outcome()
        );
    };
    assert!(
        error_text.to_ascii_lowercase().contains("resolv"),
        "expected DNS-resolution error text, got {error_text:?}"
    );
    assert_eq!(
        post.expect("worker XHR DNS failure should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_redirect_loop_reports_error_after_loadstart() {
    ensure_v8();
    let (base_url, server) = spawn_redirect_loop_http_server("/worker-xhr-loop").await;
    let url = format!("{base_url}/worker-xhr-loop");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    server
        .await
        .expect("worker xhr redirect-loop server should finish");
    let record = network.expect("worker XHR redirect loop should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("redirect limit exceeded")
    ));
    assert_eq!(
        post.expect("worker XHR redirect loop should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_cross_origin_redirect_without_cors_reports_error_after_loadstart() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_without_cors_http_servers(
            "/worker-xhr-cors-redirect-deny",
            "/worker-xhr-cors-denied-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-xhr-cors-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
        ),
        format!("{source_base_url}/worker/main.js"),
        loader,
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker XHR CORS redirect source server should finish");
    target_server
        .await
        .expect("worker XHR CORS redirect target server should finish");
    let record = network.expect("worker XHR CORS redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == crate::network_host::FAILED_ERROR_TEXT
    ));
    assert_eq!(
        post.expect("worker XHR CORS redirect should post final surface"),
        r#"{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"]}"#
    );
}

#[tokio::test]
async fn worker_xmlhttprequest_cross_origin_redirect_final_url_obeys_connect_src() {
    ensure_v8();
    let (source_base_url, _, source_server, target_server) =
        spawn_cross_origin_redirect_with_cors_http_servers(
            "/worker-xhr-csp-redirect-deny",
            "/worker-xhr-csp-target",
            "worker-xhr-csp-target",
        )
        .await;
    let url = format!("{source_base_url}/worker-xhr-csp-redirect-deny");
    let url_literal = serde_json::to_string(&url).expect("serialize worker xhr url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("worker xhr loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
        (() => {{
            const xhr = new XMLHttpRequest();
            const events = [];
            const violations = [];
            addEventListener("securitypolicyviolation", event => {{
                violations.push({{
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition,
                }});
            }});
            xhr.addEventListener('readystatechange', () => events.push('readystatechange:' + xhr.readyState));
            xhr.addEventListener('loadstart', () => events.push('loadstart'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('loadend', () => {{
                events.push('loadend');
                postMessage({{
                    readyState: xhr.readyState,
                    status: xhr.status,
                    statusText: xhr.statusText,
                    responseURL: xhr.responseURL,
                    responseText: xhr.responseText,
                    contentType: xhr.getResponseHeader('Content-Type'),
                    allHeaders: xhr.getAllResponseHeaders(),
                    events,
                    violations,
                }});
                close();
            }});
            xhr.open('GET', {url_literal});
            xhr.send();
        }})();
        "#
            ),
            format!("{source_base_url}/worker/main.js"),
        )
        .with_request_client(loader)
        .with_content_security_policies(vec!["connect-src 'self'".to_owned()]),
    );

    let mut post = None;
    let mut network = None;
    for _ in 0..2 {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::SubresourceNetwork(record) => network = Some(record),
            WorkerToParentMessage::Post(payload) => {
                post = Some(stringify_payload(&payload));
            }
            other => panic!("unexpected worker message: {other:?}"),
        }
        if post.is_some() && network.is_some() {
            break;
        }
    }

    source_server
        .await
        .expect("worker XHR CSP redirect source server should finish");
    target_server
        .await
        .expect("worker XHR CSP redirect target server should finish");
    let record = network.expect("worker XHR CSP redirect should record network failure");
    assert_eq!(record.url().as_str(), url);
    assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains("Content Security Policy")
    ));
    assert_eq!(
        post.expect("worker XHR CSP redirect should post final surface"),
        format!(
            r#"{{"readyState":4,"status":0,"statusText":"","responseURL":"","responseText":"","contentType":null,"allHeaders":"","events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"violations":[{{"blockedURI":"{url}","effectiveDirective":"connect-src","disposition":"enforce"}}]}}"#
        )
    );
}
